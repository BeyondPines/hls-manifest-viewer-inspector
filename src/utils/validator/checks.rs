use std::collections::{HashMap, HashSet};
use super::types::*;

/// Record `id` as the producer of every finding that has not already named its own check.
///
/// Findings are grouped in the report by the check that raised them, so a check that
/// returns without naming itself would land in the catch-all group.
fn produced_by(id: CheckId, mut issues: Vec<Issue>) -> Vec<Issue> {
    for issue in &mut issues {
        if issue.check_id == CheckId::Unassigned {
            issue.check_id = id;
        }
    }
    issues
}

/// RFC 8216bis §4.4.1.1 — EXTM3U must be first line
pub fn check_extm3u_header(playlists: &[MediaPlaylist]) -> Vec<Issue> {
    let mut issues = Vec::new();
    for pl in playlists {
        let first_line = pl.raw_content.lines().next().unwrap_or("");
        if first_line.trim() != "#EXTM3U" {
            issues.push(Issue {
                severity: Severity::Error,
                segment_index: -1,
                rendition_a: Some(pl.name.clone()),
                rendition_b: None,
                uri_a: None,
                uri_b: None,
                message: format!(
                    "RFC 8216bis §4.4.1.1: Playlist '{}' does not start with #EXTM3U. \
                     First line: '{}'",
                    pl.name, first_line
                ),
                uri_note: None,
                ..Default::default()
            });
        }
    }
    produced_by(CheckId::ExtM3uHeader, issues)
}

/// RFC 8216bis §4.4.1.1, §4.4.1.2, §4.4.2 — structural checks on the Multivariant Playlist.
///
/// The multivariant playlist used to be read only for its variant attributes, so a
/// multivariant playlist that was missing #EXTM3U, repeated a singleton tag or used
/// variable substitution below its declared version passed without comment.
pub fn check_master_structure(master: &MasterPlaylist) -> Vec<Issue> {
    let mut issues = Vec::new();
    let label = if master.url.is_empty() { "multivariant playlist" } else { master.url.as_str() };

    let first_line = master.raw_content.lines().next().unwrap_or("");
    if first_line.trim() != "#EXTM3U" {
        issues.push(Issue {
            severity: Severity::Error,
            check_id: CheckId::ExtM3uHeader,
            message: format!(
                "RFC 8216bis §4.4.1.1: Multivariant Playlist '{}' does not start with \
                 #EXTM3U. First line: '{}'",
                label, first_line
            ),
            uri_a: Some(master.url.clone()),
            ..Default::default()
        });
    }

    // §4.4.1.2 EXT-X-VERSION, §4.4.2.1 EXT-X-INDEPENDENT-SEGMENTS, §4.4.2.2 EXT-X-START.
    for tag in ["#EXT-X-VERSION:", "#EXT-X-INDEPENDENT-SEGMENTS", "#EXT-X-START:"] {
        let count = master.raw_content.lines().filter(|l| l.trim().starts_with(tag)).count();
        if count > 1 {
            issues.push(Issue {
                severity: Severity::Error,
                check_id: CheckId::SingletonTags,
                message: format!(
                    "RFC 8216bis §4.4.1.2/§4.4.2: Singleton tag '{}' appears {} times in \
                     Multivariant Playlist '{}'. It MUST appear at most once.",
                    tag.trim_end_matches(':'), count, label
                ),
                uri_a: Some(master.url.clone()),
                ..Default::default()
            });
        }
    }

    // §8: variable substitution requires VERSION >= 8.
    if master.version < 8 && master.raw_content.contains("#EXT-X-DEFINE:") {
        issues.push(Issue {
            severity: Severity::Error,
            check_id: CheckId::VersionCompatibility,
            message: format!(
                "RFC 8216bis §8: Multivariant Playlist '{}' uses EXT-X-DEFINE \
                 (variable substitution) which requires VERSION >= 8. Declared version: {}.",
                label, master.version
            ),
            uri_a: Some(master.url.clone()),
            ..Default::default()
        });
    }

    issues
}

/// RFC 8216bis §4.4.6.2 — every AUDIO, SUBTITLES, CLOSED-CAPTIONS and VIDEO attribute on
/// EXT-X-STREAM-INF MUST match the GROUP-ID of an EXT-X-MEDIA tag of that TYPE.
pub fn check_rendition_group_references(master: &MasterPlaylist) -> Vec<Issue> {
    let mut issues = Vec::new();

    let group_ids = |media_type: &str| -> HashSet<&str> {
        master.media_renditions.iter()
            .filter(|r| r.media_type == media_type)
            .map(|r| r.group_id.as_str())
            .collect()
    };
    let audio_groups = group_ids("AUDIO");
    let subtitle_groups = group_ids("SUBTITLES");
    let caption_groups = group_ids("CLOSED-CAPTIONS");
    let video_groups = group_ids("VIDEO");

    for v in &master.variants {
        let refs: [(&str, Option<&str>, &HashSet<&str>); 4] = [
            ("AUDIO", v.audio_group.as_deref(), &audio_groups),
            ("SUBTITLES", v.subtitle_group.as_deref(), &subtitle_groups),
            // CLOSED-CAPTIONS=NONE is an enumerated string, not a group reference.
            (
                "CLOSED-CAPTIONS",
                v.closed_captions.as_deref().filter(|c| *c != "NONE"),
                &caption_groups,
            ),
            ("VIDEO", v.video_group.as_deref(), &video_groups),
        ];
        for (attr, referenced, defined) in refs {
            let Some(group) = referenced else { continue };
            if defined.contains(group) {
                continue;
            }
            let known = {
                let mut names: Vec<&str> = defined.iter().copied().collect();
                names.sort_unstable();
                if names.is_empty() { "(none)".to_string() } else { names.join(", ") }
            };
            issues.push(Issue {
                severity: Severity::Error,
                segment_index: -1,
                rendition_a: None,
                rendition_b: None,
                uri_a: Some(v.uri.clone()),
                uri_b: None,
                message: format!(
                    "rfc8216bis §4.4.6.2: EXT-X-STREAM-INF for URI '{}' has {}=\"{}\" but no \
                     EXT-X-MEDIA tag with TYPE={} declares that GROUP-ID. The attribute value \
                     MUST match the GROUP-ID of a Rendition Group of that type \
                     (declared {} groups: {}).",
                    v.uri, attr, group, attr, attr, known
                ),
                uri_note: None,
                ..Default::default()
            });
        }
    }

    produced_by(CheckId::RenditionGroupReferences, issues)
}

/// RFC 8216bis §4.4.3.1 — TARGETDURATION presence, per-segment compliance, and accuracy.
/// The spec says the EXTINF duration "when rounded to the nearest integer, MUST be less than
/// or equal to the Target Duration." (round-half-away-from-zero, matching common rounding.)
pub fn check_target_duration_compliance(playlists: &[MediaPlaylist]) -> Vec<Issue> {
    let mut issues = Vec::new();
    for pl in playlists {
        if pl.target_duration <= 0.0 {
            issues.push(Issue {
                severity: Severity::Error,
                segment_index: -1,
                rendition_a: Some(pl.name.clone()),
                rendition_b: None,
                uri_a: None,
                uri_b: None,
                message: format!(
                    "rfc8216bis §4.4.3.1: EXT-X-TARGETDURATION missing or zero in '{}'. \
                     Every Media Playlist MUST declare a positive TARGETDURATION.",
                    pl.name
                ),
                uri_note: None,
                ..Default::default()
            });
            continue;
        }
        let target_int = pl.target_duration as u64;
        for (idx, seg) in pl.segments.iter().enumerate() {
            // §4.4.3.1: "rounded to the nearest integer" — not ceil
            let rounded = seg.duration.round() as u64;
            if rounded > target_int {
                issues.push(Issue {
                    severity: Severity::Error,
                    segment_index: idx as i32,
                    rendition_a: Some(pl.name.clone()),
                    rendition_b: None,
                    uri_a: Some(seg.uri.clone()),
                    uri_b: None,
                    message: format!(
                        "rfc8216bis §4.4.3.1: Segment {} in '{}' duration {:.6}s \
                         (round={}s) exceeds TARGETDURATION {}s.",
                        idx, pl.name, seg.duration, rounded, target_int
                    ),
                    uri_note: None,
                    ..Default::default()
                });
            }
        }
        if let Some(max_extinf) = pl.segments.iter().map(|s| s.duration).reduce(f64::max) {
            let rounded_max = max_extinf.round() as u64;
            // If declared TARGETDURATION exceeds the rounded longest segment by more than 1s, warn
            if target_int > rounded_max + 1 {
                issues.push(Issue {
                    severity: Severity::Warn,
                    segment_index: -1,
                    rendition_a: Some(pl.name.clone()),
                    rendition_b: None,
                    uri_a: None,
                    uri_b: None,
                    message: format!(
                        "rfc8216bis §4.4.3.1: TARGETDURATION={}s in '{}' is more than 1s \
                         above the longest segment (longest={:.6}s, round={}s). \
                         Consider reducing TARGETDURATION for accuracy.",
                        target_int, pl.name, max_extinf, rounded_max
                    ),
                    uri_note: None,
                    ..Default::default()
                });
            }
        }
    }
    produced_by(CheckId::TargetDurationCompliance, issues)
}

/// RFC 8216bis §6.2.4 — PDT coverage: if any segment has PDT, all should
pub fn check_pdt_coverage(playlists: &[MediaPlaylist]) -> Vec<Issue> {
    let mut issues = Vec::new();
    for pl in playlists {
        if pl.segments.is_empty() {
            continue;
        }
        let has_any_pdt = pl.segments.iter().any(|s| s.pdt.is_some());
        let all_have_pdt = pl.segments.iter().all(|s| s.pdt.is_some());
        if has_any_pdt && !all_have_pdt {
            let missing_count = pl.segments.iter().filter(|s| s.pdt.is_none()).count();
            issues.push(Issue {
                severity: Severity::Warn,
                segment_index: -1,
                rendition_a: Some(pl.name.clone()),
                rendition_b: None,
                uri_a: None,
                uri_b: None,
                message: format!(
                    "RFC 8216bis §6.2.4: Playlist '{}' has partial PDT coverage: \
                     {} of {} segments lack EXT-X-PROGRAM-DATE-TIME.",
                    pl.name, missing_count, pl.segments.len()
                ),
                uri_note: None,
                ..Default::default()
            });
        }
    }
    produced_by(CheckId::PdtCoverage, issues)
}

/// RFC 8216bis §4.4.1.2, §4.4.2, §4.4.3 — Singleton tags must not appear more than once.
/// Covers: EXT-X-VERSION (§4.4.1.2), EXT-X-INDEPENDENT-SEGMENTS (§4.4.2.1),
/// EXT-X-START (§4.4.2.2), and all Media Playlist singleton tags (§4.4.3).
pub fn check_media_sequence_duplicate_tags(playlists: &[MediaPlaylist]) -> Vec<Issue> {
    let mut issues = Vec::new();
    let singleton_tags = [
        "#EXT-X-VERSION:",
        "#EXT-X-INDEPENDENT-SEGMENTS",
        "#EXT-X-START:",
        "#EXT-X-TARGETDURATION:",
        "#EXT-X-MEDIA-SEQUENCE:",
        "#EXT-X-DISCONTINUITY-SEQUENCE:",
        "#EXT-X-PLAYLIST-TYPE:",
        "#EXT-X-I-FRAMES-ONLY",
        "#EXT-X-PART-INF:",
        "#EXT-X-SERVER-CONTROL:",
    ];
    for pl in playlists {
        for tag in &singleton_tags {
            let count = pl.raw_content.lines().filter(|l| l.trim().starts_with(tag)).count();
            if count > 1 {
                issues.push(Issue {
                    severity: Severity::Error,
                    segment_index: -1,
                    rendition_a: Some(pl.name.clone()),
                    rendition_b: None,
                    uri_a: None,
                    uri_b: None,
                    message: format!(
                        "RFC 8216bis §4.4.1.2/§4.4.3: Singleton tag '{}' appears {} times in \
                         '{}'. It MUST appear at most once.",
                        tag.trim_end_matches(':'), count, pl.name
                    ),
                    uri_note: None,
                    ..Default::default()
                });
            }
        }
    }
    produced_by(CheckId::SingletonTags, issues)
}

/// RFC 8216bis §4.4.6.2, §6.2.4 — consistency of EXT-X-STREAM-INF tags that share a URI.
///
/// A Variant Stream is the combination of a URI and the Rendition Groups it pairs with, so
/// two STREAM-INF tags that share a URI but name different AUDIO, SUBTITLES or
/// CLOSED-CAPTIONS groups are different Variant Streams and their CODECS and BANDWIDTH are
/// expected to differ. Only entries that agree on all of those attributes are compared for
/// CODECS and BANDWIDTH, and a mismatch there is reported as a warning: the spec's
/// requirement is on what the attributes describe, and a player reads them per entry.
///
/// The video codec is different. §6.2.4 requires every Variant Stream of a presentation to
/// carry the same video encoding, so the same media URI described with two different video
/// codecs is a contradiction no choice of audio can explain, and that stays an error.
pub fn check_stream_inf_consistency(master: &MasterPlaylist) -> Vec<Issue> {
    let mut issues = Vec::new();

    // URI plus everything that makes two STREAM-INF entries the same Variant Stream.
    type VariantKey<'a> =
        (&'a str, Option<&'a str>, Option<&'a str>, Option<&'a str>, Option<&'a str>);
    fn variant_key(v: &MasterRendition) -> VariantKey<'_> {
        (
            v.uri.as_str(),
            v.audio_group.as_deref(),
            v.subtitle_group.as_deref(),
            v.closed_captions.as_deref(),
            v.video_group.as_deref(),
        )
    }
    // First comma-separated token of a CODECS string: the video codec.
    fn video_codec(codecs: &str) -> &str {
        codecs.split(',').next().unwrap_or(codecs).trim()
    }

    let mut by_variant: HashMap<VariantKey<'_>, Vec<&MasterRendition>> = HashMap::new();
    let mut by_uri: HashMap<&str, Vec<&MasterRendition>> = HashMap::new();
    for v in &master.variants {
        by_variant.entry(variant_key(v)).or_default().push(v);
        by_uri.entry(v.uri.as_str()).or_default().push(v);
    }

    for ((uri, audio, subtitles, captions, video), variants) in &by_variant {
        if variants.len() < 2 {
            continue;
        }
        let groups = format!(
            "AUDIO={}, SUBTITLES={}, CLOSED-CAPTIONS={}, VIDEO={}",
            audio.unwrap_or("(none)"),
            subtitles.unwrap_or("(none)"),
            captions.unwrap_or("(none)"),
            video.unwrap_or("(none)")
        );

        let codecs_set: HashSet<Option<&str>> = variants.iter().map(|v| v.codecs.as_deref()).collect();
        if codecs_set.len() > 1 {
            let details: Vec<String> = variants.iter()
                .map(|v| format!("CODECS={}", v.codecs.as_deref().unwrap_or("(none)")))
                .collect();
            issues.push(Issue {
                severity: Severity::Warn,
                uri_a: Some((*uri).to_string()),
                message: format!(
                    "rfc8216bis §4.4.6.2: Multiple EXT-X-STREAM-INF tags share URI '{}' and the \
                     same Rendition Groups ({}) but declare different CODECS values: {}. \
                     One of them misdescribes what the URI contains.",
                    uri, groups, details.join(", ")
                ),
                ..Default::default()
            });
        }

        let bw_set: HashSet<Option<u64>> = variants.iter().map(|v| v.bandwidth).collect();
        if bw_set.len() > 1 {
            let details: Vec<String> = variants.iter()
                .map(|v| format!(
                    "BANDWIDTH={}",
                    v.bandwidth.map_or("(none)".to_string(), |b| b.to_string())
                ))
                .collect();
            issues.push(Issue {
                severity: Severity::Warn,
                uri_a: Some((*uri).to_string()),
                message: format!(
                    "rfc8216bis §4.4.6.2: Multiple EXT-X-STREAM-INF tags share URI '{}' and the \
                     same Rendition Groups ({}) but declare different BANDWIDTH values: {}. \
                     BANDWIDTH is the peak bit rate of the same media in each case.",
                    uri, groups, details.join(", ")
                ),
                ..Default::default()
            });
        }
    }

    // Across all entries for a URI, the video codec cannot depend on the audio group.
    for (uri, variants) in &by_uri {
        if variants.len() < 2 {
            continue;
        }
        let video_codecs: HashSet<&str> = variants.iter()
            .filter_map(|v| v.codecs.as_deref())
            .map(video_codec)
            .collect();
        if video_codecs.len() > 1 {
            let details: Vec<String> = variants.iter()
                .map(|v| format!(
                    "AUDIO={} CODECS={}",
                    v.audio_group.as_deref().unwrap_or("(none)"),
                    v.codecs.as_deref().unwrap_or("(none)")
                ))
                .collect();
            issues.push(Issue {
                severity: Severity::Error,
                uri_a: Some((*uri).to_string()),
                message: format!(
                    "rfc8216bis §6.2.4: Multiple EXT-X-STREAM-INF tags share URI '{}' but \
                     declare different video codecs: {}. The same media cannot be encoded two \
                     ways, and every Variant Stream of a presentation MUST use the same video \
                     encoding.",
                    uri, details.join(", ")
                ),
                ..Default::default()
            });
        }
    }

    produced_by(CheckId::StreamInfConsistency, issues)
}

/// RFC 8216bis §4.4.6.2 — BANDWIDTH is a REQUIRED attribute on EXT-X-STREAM-INF.
pub fn check_bandwidth_required(master: &MasterPlaylist) -> Vec<Issue> {
    let mut issues = Vec::new();
    for v in &master.variants {
        if v.bandwidth.is_none() {
            issues.push(Issue {
                severity: Severity::Error,
                segment_index: -1,
                rendition_a: None,
                rendition_b: None,
                uri_a: Some(v.uri.clone()),
                uri_b: None,
                message: format!(
                    "RFC 8216bis §4.4.6.2: EXT-X-STREAM-INF for URI '{}' is missing \
                     the BANDWIDTH attribute, which is REQUIRED.",
                    v.uri
                ),
                uri_note: None,
                ..Default::default()
            });
        }
    }
    produced_by(CheckId::BandwidthRequired, issues)
}

/// RFC 8216bis §4.4.6.1.1 — When a Playlist contains multiple Groups of the same TYPE,
/// every Group MUST contain the same set of member NAMEs.
pub fn check_media_group_membership(master: &MasterPlaylist) -> Vec<Issue> {
    let mut issues = Vec::new();

    // Collect all distinct media types that appear in more than one group
    let mut by_type: HashMap<&str, HashMap<&str, Vec<&MediaRendition>>> = HashMap::new();
    for r in &master.media_renditions {
        by_type
            .entry(r.media_type.as_str())
            .or_default()
            .entry(r.group_id.as_str())
            .or_default()
            .push(r);
    }

    for (media_type, groups) in &by_type {
        if groups.len() < 2 {
            continue; // only one group of this type — nothing to compare
        }

        // Build the union of member NAMEs across all groups of this type
        let all_names: HashSet<&str> = groups.values()
            .flat_map(|members| members.iter().map(|r| r.name.as_str()))
            .collect();

        for (group_id, members) in groups {
            let present: HashSet<&str> = members.iter().map(|r| r.name.as_str()).collect();
            let mut missing: Vec<&str> = all_names.difference(&present).copied().collect();
            if missing.is_empty() {
                continue;
            }
            missing.sort_unstable();
            issues.push(Issue {
                severity: Severity::Error,
                segment_index: -1,
                rendition_a: None,
                rendition_b: None,
                uri_a: None,
                uri_b: None,
                message: format!(
                    "rfc8216bis §4.4.6.1.1: {} Group '{}' is missing member(s) present in \
                     other groups of the same type: {}. All groups of the same TYPE MUST \
                     have the same set of members.",
                    media_type, group_id,
                    missing.iter().map(|n| format!("'{}'", n)).collect::<Vec<_>>().join(", ")
                ),
                uri_note: None,
                ..Default::default()
            });
        }
    }
    produced_by(CheckId::MediaGroupMembership, issues)
}

/// RFC 8216bis §8 — Version compatibility
pub fn check_version_compatibility(playlists: &[MediaPlaylist]) -> Vec<Issue> {
    let mut issues = Vec::new();
    for pl in playlists {
        let v = pl.version;
        let content = &pl.raw_content;
        // EXT-X-KEY with IV requires v2+
        if v < 2 && content.contains("#EXT-X-KEY:") && content.contains("IV=") {
            issues.push(Issue {
                severity: Severity::Error,
                segment_index: -1,
                rendition_a: Some(pl.name.clone()),
                rendition_b: None,
                uri_a: None,
                uri_b: None,
                message: format!(
                    "RFC 8216bis §8: '{}' uses EXT-X-KEY with IV attribute \
                     which requires VERSION >= 2. Declared version: {}.",
                    pl.name, v
                ),
                uri_note: None,
                ..Default::default()
            });
        }
        // Floating-point EXTINF requires v3+
        if v < 3 {
            for seg in &pl.segments {
                if seg.duration.fract() != 0.0 {
                    issues.push(Issue {
                        severity: Severity::Error,
                        segment_index: -1,
                        rendition_a: Some(pl.name.clone()),
                        rendition_b: None,
                        uri_a: None,
                        uri_b: None,
                        message: format!(
                            "RFC 8216bis §8: '{}' uses floating-point EXTINF ({:.3}s) \
                             which requires VERSION >= 3. Declared version: {}.",
                            pl.name, seg.duration, v
                        ),
                        uri_note: None,
                        ..Default::default()
                    });
                    break;
                }
            }
        }
        // EXT-X-BYTERANGE requires v4+
        if v < 4 && content.contains("#EXT-X-BYTERANGE:") {
            issues.push(Issue {
                severity: Severity::Error,
                segment_index: -1,
                rendition_a: Some(pl.name.clone()),
                rendition_b: None,
                uri_a: None,
                uri_b: None,
                message: format!(
                    "RFC 8216bis §8: '{}' uses EXT-X-BYTERANGE \
                     which requires VERSION >= 4. Declared version: {}.",
                    pl.name, v
                ),
                uri_note: None,
                ..Default::default()
            });
        }
        // §8: EXT-X-MAP needs v6 on its own, and v5 in an I-frames-only playlist.
        if content.contains("#EXT-X-MAP:") {
            let iframes_only = content.contains("#EXT-X-I-FRAMES-ONLY");
            let required = if iframes_only { 5 } else { 6 };
            if v < required {
                issues.push(Issue {
                    severity: Severity::Error,
                    segment_index: -1,
                    rendition_a: Some(pl.name.clone()),
                    rendition_b: None,
                    uri_a: None,
                    uri_b: None,
                    message: format!(
                        "RFC 8216bis §8: '{}' uses EXT-X-MAP {} which requires \
                         VERSION >= {}. Declared version: {}.",
                        pl.name,
                        if iframes_only {
                            "in a playlist with EXT-X-I-FRAMES-ONLY"
                        } else {
                            "without EXT-X-I-FRAMES-ONLY"
                        },
                        required, v
                    ),
                    uri_note: None,
                    ..Default::default()
                });
            }
        }
        // EXT-X-SKIP requires v9+
        if v < 9 && content.contains("#EXT-X-SKIP:") {
            issues.push(Issue {
                severity: Severity::Error,
                segment_index: -1,
                rendition_a: Some(pl.name.clone()),
                rendition_b: None,
                uri_a: None,
                uri_b: None,
                message: format!(
                    "RFC 8216bis §8: '{}' uses EXT-X-SKIP \
                     which requires VERSION >= 9. Declared version: {}.",
                    pl.name, v
                ),
                uri_note: None,
                ..Default::default()
            });
        }
    }
    produced_by(CheckId::VersionCompatibility, issues)
}

/// RFC 8216 §6.2.2 — Live playlists must have >= 3 segments
pub fn check_live_playlist_min_segments(playlists: &[MediaPlaylist]) -> Vec<Issue> {
    let mut issues = Vec::new();
    for pl in playlists {
        if pl.has_endlist {
            continue;
        }
        let n = pl.segments.len();
        if n < 3 {
            issues.push(Issue {
                severity: Severity::Error,
                segment_index: -1,
                rendition_a: Some(pl.name.clone()),
                rendition_b: None,
                uri_a: None,
                uri_b: None,
                message: format!(
                    "RFC 8216 §6.2.2: Live playlist '{}' contains only {} segment(s). \
                     A Live Media Playlist MUST retain at least 3 segments.",
                    pl.name, n
                ),
                uri_note: None,
                ..Default::default()
            });
        }
    }
    produced_by(CheckId::LivePlaylistWindow, issues)
}


/// RFC 8216bis §6.2.4 — every Media Playlist of a presentation MUST have the same
/// TARGETDURATION, so that a client switching renditions keeps the same reload interval.
///
/// The spec's own exception is for trick-play: an I-frame playlist with
/// EXT-X-PLAYLIST-TYPE:VOD may use a different target duration, so those renditions are
/// excluded from the comparison rather than counted against it.
pub fn check_targetduration_consistency(playlists: &[MediaPlaylist]) -> Vec<Issue> {
    let mut issues = Vec::new();
    if playlists.len() < 2 {
        return issues;
    }
    let is_vod_iframe_playlist = |pl: &MediaPlaylist| {
        (pl.iframes_only || pl.is_iframe) && pl.playlist_type.as_deref() == Some("VOD")
    };
    let td_values: Vec<(&str, f64)> = playlists.iter()
        .filter(|pl| pl.target_duration > 0.0 && !is_vod_iframe_playlist(pl))
        .map(|pl| (pl.name.as_str(), pl.target_duration))
        .collect();
    if td_values.len() < 2 {
        return issues;
    }
    let unique_tds: HashSet<u64> = td_values.iter().map(|(_, v)| *v as u64).collect();
    if unique_tds.len() > 1 {
        let td_summary: Vec<String> = td_values.iter()
            .map(|(name, td)| format!("{}={:.0}s", name, td))
            .collect();
        issues.push(Issue::error(format!(
            "rfc8216bis §6.2.4: EXT-X-TARGETDURATION values differ across renditions \
             ({} distinct values). Every Media Playlist of a presentation MUST declare the \
             same TARGETDURATION, apart from I-frame playlists with PLAYLIST-TYPE:VOD, which \
             are excluded here. Details: {}",
            unique_tds.len(), td_summary.join(", ")
        )));
    }
    produced_by(CheckId::TargetDurationConsistency, issues)
}

/// RFC 8216bis §4.4.3.5 — PLAYLIST-TYPE / ENDLIST consistency.
/// VOD playlists MUST have EXT-X-ENDLIST. EVENT playlists are valid live
/// playlists that grow (segments can only be appended, never removed) and
/// also MUST have EXT-X-ENDLIST when the event is complete.
pub fn check_playlist_type_endlist(playlists: &[MediaPlaylist]) -> Vec<Issue> {
    let mut issues = Vec::new();
    for pl in playlists {
        match pl.playlist_type.as_deref() {
            Some("VOD") if !pl.has_endlist => {
                issues.push(Issue {
                    severity: Severity::Error,
                    segment_index: -1,
                    rendition_a: Some(pl.name.clone()),
                    rendition_b: None,
                    uri_a: None,
                    uri_b: None,
                    message: format!(
                        "RFC 8216bis §4.4.3.5: Playlist '{}' declares PLAYLIST-TYPE:VOD \
                         but is missing EXT-X-ENDLIST. A VOD playlist MUST end with EXT-X-ENDLIST.",
                        pl.name
                    ),
                    uri_note: None,
                    ..Default::default()
                });
            }
            // EVENT playlists are live; EXT-X-ENDLIST is added when the event ends.
            // An EVENT playlist without ENDLIST is valid and expected during a live event.
            Some("EVENT") => { /* valid — no error */ }
            _ => {}
        }
    }
    produced_by(CheckId::PlaylistTypeEndlist, issues)
}

/// RFC 8216bis §4.4.4.4 — Encryption consistency across renditions.
///
/// Only playlists that declare an EXT-X-KEY are compared, so a rendition with no EXT-X-KEY at
/// all is absent from the comparison rather than being counted as METHOD=NONE.
pub fn check_encryption_consistency(playlists: &[MediaPlaylist]) -> Vec<Issue> {
    let mut issues = Vec::new();
    let method_map: HashMap<&str, &HashSet<String>> = playlists.iter()
        .filter(|pl| !pl.encryption_methods.is_empty())
        .map(|pl| (pl.name.as_str(), &pl.encryption_methods))
        .collect();
    if method_map.is_empty() {
        return issues;
    }
    // Per-playlist: mixed encryption
    for (&name, methods) in &method_map {
        if methods.len() > 1 {
            let is_normal_mix = (methods.contains("AES-128") && methods.contains("NONE") && methods.len() == 2)
                || (methods.contains("SAMPLE-AES") && methods.contains("NONE") && methods.len() == 2);
            if !is_normal_mix {
                let sorted: Vec<&String> = methods.iter().collect();
                issues.push(Issue::warn(format!(
                    "RFC 8216bis §4.4.4.4: Playlist '{}' uses multiple encryption methods: {:?}.",
                    name, sorted
                )));
            }
        }
    }
    // Cross-rendition consistency
    let mut all_methods: HashSet<&str> = HashSet::new();
    for methods in method_map.values() {
        for m in *methods {
            if m != "NONE" || methods.len() == 1 {
                all_methods.insert(m.as_str());
            }
        }
    }
    // Renditions that differ in encryption method are worth reporting, but this is a
    // warning: the spec's requirement is that each Media Segment can be decrypted from its
    // own playlist, and a presentation that encrypts video while leaving, say, a subtitle
    // or I-frame rendition clear is deployed deliberately and plays.
    if all_methods.len() > 1 {
        let mut sorted: Vec<&str> = all_methods.iter().copied().collect();
        sorted.sort_unstable();
        issues.push(Issue::warn(format!(
            "rfc8216bis §4.4.4.4: Renditions declare different EXT-X-KEY METHOD values: {}. \
             Check this is intended — a client that can decrypt one rendition may not be able \
             to switch to another.",
            sorted.join(", ")
        )));
    }
    produced_by(CheckId::EncryptionConsistency, issues)
}

/// RFC 8216bis §6.2.4 — EXT-X-DISCONTINUITY-SEQUENCE MUST match across the renditions of a
/// presentation, because a client synchronises renditions by discontinuity sequence number
/// before it can align their timelines.
pub fn check_discontinuity_sequence(playlists: &[MediaPlaylist]) -> Vec<Issue> {
    let mut issues = Vec::new();
    if playlists.len() < 2 {
        return issues;
    }
    let disc_seqs: HashSet<u64> = playlists.iter().map(|pl| pl.discontinuity_sequence).collect();
    if disc_seqs.len() > 1 {
        let details: Vec<String> = playlists.iter()
            .map(|pl| format!("{}={}", pl.name, pl.discontinuity_sequence))
            .collect();
        issues.push(Issue::error(format!(
            "rfc8216bis §6.2.4: EXT-X-DISCONTINUITY-SEQUENCE values differ across renditions: \
             {}. Renditions of the same presentation MUST carry matching discontinuity \
             sequence numbers so clients can align their timelines.",
            details.join(", ")
        )));
    }
    produced_by(CheckId::DiscontinuitySequence, issues)
}

/// Segment count comparison across renditions (non-live)
pub fn check_segment_count(playlists: &[MediaPlaylist]) -> Vec<Issue> {
    let mut issues = Vec::new();
    if playlists.len() < 2 {
        return issues;
    }
    // Only compare VOD VIDEO renditions — audio encoders produce different segment
    // counts than video encoders even for the same content duration; comparing
    // across media types always produces false positives.
    let vod_playlists: Vec<&MediaPlaylist> = playlists.iter()
        .filter(|pl| pl.has_endlist && pl.media_type == "VIDEO" && !pl.is_iframe)
        .collect();
    if vod_playlists.len() < 2 {
        return issues;
    }
    let counts: HashSet<usize> = vod_playlists.iter().map(|pl| pl.segments.len()).collect();
    if counts.len() > 1 {
        let details: Vec<String> = vod_playlists.iter()
            .map(|pl| format!("{}={}", pl.name, pl.segments.len()))
            .collect();
        issues.push(Issue::warn(format!(
            "Segment count mismatch across VIDEO renditions: {}. \
             All video renditions should have the same number of segments.",
            details.join(", ")
        )));
    }
    produced_by(CheckId::SegmentCount, issues)
}

/// EXTINF duration drift between renditions (MSN-aligned)
/// Only compares VIDEO vs VIDEO — audio encoders produce different segment
/// boundaries than video encoders, making cross-type drift meaningless.
pub fn check_duration_drift(playlists: &[MediaPlaylist], tolerance_ms: f64) -> Vec<Issue> {
    let mut issues = Vec::new();
    if playlists.len() < 2 {
        return issues;
    }
    let tolerance_s = tolerance_ms / 1000.0;
    for i in 0..playlists.len() {
        for j in (i + 1)..playlists.len() {
            let pl_a = &playlists[i];
            let pl_b = &playlists[j];
            // Skip cross-type pairs and I-frame-only playlists
            if pl_a.media_type != pl_b.media_type
                || pl_a.media_type != "VIDEO"
                || pl_a.is_iframe || pl_b.is_iframe
            {
                continue;
            }
            for (seg_a, seg_b, msn) in overlapping_segments(pl_a, pl_b) {
                let diff = (seg_a.duration - seg_b.duration).abs();
                if diff > tolerance_s {
                    issues.push(Issue {
                        severity: Severity::Warn,
                        segment_index: msn as i32,
                        rendition_a: Some(pl_a.name.clone()),
                        rendition_b: Some(pl_b.name.clone()),
                        uri_a: Some(seg_a.uri.clone()),
                        uri_b: Some(seg_b.uri.clone()),
                        message: format!(
                            "EXTINF drift at MSN {}: '{}' has {:.3}s vs '{}' has {:.3}s \
                             (diff={:.3}s, tolerance={:.3}s).",
                            msn, pl_a.name, seg_a.duration, pl_b.name, seg_b.duration,
                            diff, tolerance_s
                        ),
                        uri_note: None,
                        ..Default::default()
                    });
                }
            }
        }
    }
    produced_by(CheckId::DurationDrift, issues)
}

/// PDT alignment across renditions (MSN-aligned)
/// Only compares VIDEO vs VIDEO — audio PDT is extrapolated from audio segment
/// durations which differ from video, causing apparent drift that is not real.
pub fn check_pdt_alignment(playlists: &[MediaPlaylist], tolerance_ms: f64) -> Vec<Issue> {
    let mut issues = Vec::new();
    if playlists.len() < 2 {
        return issues;
    }
    let tolerance_s = tolerance_ms / 1000.0;
    for i in 0..playlists.len() {
        for j in (i + 1)..playlists.len() {
            let pl_a = &playlists[i];
            let pl_b = &playlists[j];
            // Skip cross-type pairs and I-frame-only playlists
            if pl_a.media_type != pl_b.media_type
                || pl_a.media_type != "VIDEO"
                || pl_a.is_iframe || pl_b.is_iframe
            {
                continue;
            }
            for (seg_a, seg_b, msn) in overlapping_segments(pl_a, pl_b) {
                if let (Some(pdt_a), Some(pdt_b)) = (seg_a.pdt, seg_b.pdt) {
                    let diff = (pdt_a - pdt_b).abs();
                    if diff > tolerance_s {
                        issues.push(Issue {
                            severity: Severity::Warn,
                            segment_index: msn as i32,
                            rendition_a: Some(pl_a.name.clone()),
                            rendition_b: Some(pl_b.name.clone()),
                            uri_a: Some(seg_a.uri.clone()),
                            uri_b: Some(seg_b.uri.clone()),
                            message: format!(
                                "PDT misalignment at MSN {}: diff={:.3}s (tolerance={:.3}s).",
                                msn, diff, tolerance_s
                            ),
                            uri_note: None,
                            ..Default::default()
                        });
                    }
                }
            }
        }
    }
    produced_by(CheckId::PdtAlignment, issues)
}

/// Cumulative EXTINF drift across renditions
/// Only compares VIDEO renditions — comparing video total to audio total is
/// meaningless since they use different segment boundaries.
///
/// Uses the MSN-aligned common window across all renditions so that concurrent live fetches
/// (which may return one more or fewer segment per rendition) do not trigger false positives.
pub fn check_cumulative_drift(playlists: &[MediaPlaylist], tolerance_ms: f64) -> Vec<Issue> {
    let mut issues = Vec::new();
    let video_pls: Vec<&MediaPlaylist> = playlists.iter()
        .filter(|pl| pl.media_type == "VIDEO" && !pl.is_iframe)
        .collect();
    if video_pls.len() < 2 {
        return issues;
    }
    let tolerance_s = tolerance_ms / 1000.0;

    // Find the MSN range that every VIDEO rendition has in common.
    // This eliminates false positives from concurrent live fetches where one rendition
    // arrives with one extra segment, adding ~TARGETDURATION of spurious drift.
    let mut overlap_start = 0u64;
    let mut overlap_end = u64::MAX;
    for pl in &video_pls {
        let start = pl.media_sequence + pl.skipped_segments;
        let end = start + pl.segments.len() as u64;
        overlap_start = overlap_start.max(start);
        overlap_end = overlap_end.min(end);
    }
    if overlap_start >= overlap_end {
        // No shared window at all — renditions are completely disjoint; skip.
        return issues;
    }
    let window_size = (overlap_end - overlap_start) as usize;

    // Sum EXTINF for each rendition over the common window only.
    let totals: Vec<(&str, f64)> = video_pls.iter().map(|pl| {
        let pl_start = pl.media_sequence + pl.skipped_segments;
        let sum: f64 = (overlap_start..overlap_end)
            .filter_map(|msn| {
                let idx = (msn - pl_start) as usize;
                pl.segments.get(idx).map(|s| s.duration)
            })
            .sum();
        (pl.name.as_str(), sum)
    }).collect();

    let max_total = totals.iter().map(|(_, t)| *t).fold(f64::NEG_INFINITY, f64::max);
    let min_total = totals.iter().map(|(_, t)| *t).fold(f64::INFINITY, f64::min);
    let drift = max_total - min_total;
    if drift > tolerance_s {
        let details: Vec<String> = totals.iter()
            .map(|(name, total)| format!("{}={:.3}s", name, total))
            .collect();
        issues.push(Issue::warn(format!(
            "Cumulative EXTINF drift across renditions: {:.3}s (tolerance={:.3}s) \
             over {} common segments (MSN {}-{}). Totals: {}",
            drift, tolerance_s, window_size, overlap_start, overlap_end - 1,
            details.join(", ")
        )));
    }
    produced_by(CheckId::CumulativeDrift, issues)
}

/// MSN-aligned segment pairing helper
fn overlapping_segments<'a>(
    pl_a: &'a MediaPlaylist,
    pl_b: &'a MediaPlaylist,
) -> Vec<(&'a Segment, &'a Segment, u64)> {
    let mut pairs = Vec::new();
    let start_a = pl_a.media_sequence + pl_a.skipped_segments;
    let end_a = start_a + pl_a.segments.len() as u64;
    let start_b = pl_b.media_sequence + pl_b.skipped_segments;
    let end_b = start_b + pl_b.segments.len() as u64;
    let overlap_start = start_a.max(start_b);
    let overlap_end = end_a.min(end_b);
    for msn in overlap_start..overlap_end {
        let idx_a = (msn - start_a) as usize;
        let idx_b = (msn - start_b) as usize;
        if idx_a < pl_a.segments.len() && idx_b < pl_b.segments.len() {
            pairs.push((&pl_a.segments[idx_a], &pl_b.segments[idx_b], msn));
        }
    }
    pairs
}

/// LL-HLS compliance checks (draft-pantos-hls-rfc8216bis §4.4.3–4.4.5, §6.2.5.2)
pub fn check_ll_hls_compliance(playlists: &[MediaPlaylist]) -> Vec<Issue> {
    let mut issues = Vec::new();

    // ── Per-rendition checks ──────────────────────────────────────────────────
    for pl in playlists {
        let has_parts = !pl.parts.is_empty();
        let has_part_inf = pl.part_target.is_some();
        let has_server_control = pl.server_control.is_some();
        if !has_parts && !has_part_inf && !has_server_control {
            continue;
        }

        // 1. EXT-X-PART-INF / PART-TARGET required when parts exist
        if has_parts && !has_part_inf {
            issues.push(Issue {
                severity: Severity::Error,
                segment_index: -1,
                rendition_a: Some(pl.name.clone()),
                rendition_b: None,
                uri_a: None,
                uri_b: None,
                message: format!(
                    "LL-HLS §6.2.5.2: '{}' contains EXT-X-PART tags but no EXT-X-PART-INF. \
                     PART-TARGET is required.",
                    pl.name
                ),
                uri_note: None,
                ..Default::default()
            });
        }

        // 2. PART-HOLD-BACK is REQUIRED once EXT-X-PART-INF is present (§4.4.3.8): it is
        //    what tells a client how far from the live edge it may start playing parts.
        if has_part_inf {
            let has_part_hold_back = pl.server_control.as_ref()
                .is_some_and(|sc| sc.part_hold_back.is_some());
            if !has_part_hold_back {
                issues.push(Issue {
                    severity: Severity::Error,
                    segment_index: -1,
                    rendition_a: Some(pl.name.clone()),
                    rendition_b: None,
                    uri_a: None,
                    uri_b: None,
                    message: format!(
                        "rfc8216bis §4.4.3.8: '{}' declares EXT-X-PART-INF but \
                         EXT-X-SERVER-CONTROL has no PART-HOLD-BACK. The attribute is REQUIRED \
                         when the playlist contains EXT-X-PART-INF.",
                        pl.name
                    ),
                    uri_note: None,
                    ..Default::default()
                });
            }
        }

        // 3. Part durations must not exceed PART-TARGET
        if let Some(pt) = pl.part_target {
            for (idx, part) in pl.parts.iter().enumerate() {
                if part.duration > pt + 0.001 {
                    issues.push(Issue {
                        severity: Severity::Error,
                        segment_index: idx as i32,
                        rendition_a: Some(pl.name.clone()),
                        rendition_b: None,
                        uri_a: Some(part.uri.clone()),
                        uri_b: None,
                        message: format!(
                            "LL-HLS §4.4.4.9: Part {} in '{}' has duration {:.5}s exceeding \
                             PART-TARGET {:.5}s.",
                            idx, pl.name, part.duration, pt
                        ),
                        uri_note: None,
                        ..Default::default()
                    });
                }
            }
        }

        // 4. EXT-X-PRELOAD-HINT with TYPE=PART should be present at playlist tail
        if has_parts {
            let has_part_hint = pl.preload_hint_uri.is_some()
                && pl.preload_hint_type.as_deref() == Some("PART");
            if !has_part_hint {
                issues.push(Issue {
                    severity: Severity::Warn,
                    segment_index: -1,
                    rendition_a: Some(pl.name.clone()),
                    rendition_b: None,
                    uri_a: None,
                    uri_b: None,
                    message: format!(
                        "LL-HLS §4.4.5.3: '{}' is missing EXT-X-PRELOAD-HINT with TYPE=PART \
                         at the playlist tail. Clients cannot prefetch the next partial segment.",
                        pl.name
                    ),
                    uri_note: None,
                    ..Default::default()
                });
            }
        }

        // 6. EXT-X-RENDITION-REPORT should be present
        if has_parts && pl.rendition_reports.is_empty() {
            issues.push(Issue {
                severity: Severity::Warn,
                segment_index: -1,
                rendition_a: Some(pl.name.clone()),
                rendition_b: None,
                uri_a: None,
                uri_b: None,
                message: format!(
                    "LL-HLS §4.4.5.4: '{}' has no EXT-X-RENDITION-REPORT tags. \
                     Each media playlist should report the last MSN/Part of every \
                     other rendition so clients can switch without extra fetches.",
                    pl.name
                ),
                uri_note: None,
                ..Default::default()
            });
        }

        // 7. SERVER-CONTROL: CAN-SKIP-UNTIL MUST be >= 6× TARGETDURATION (§4.4.3.8)
        if let Some(sc) = &pl.server_control
            && let Some(csu) = sc.can_skip_until
                && pl.target_duration > 0.0 && csu < pl.target_duration * 6.0 - 0.001 {
                    issues.push(Issue {
                        severity: Severity::Error,
                        segment_index: -1,
                        rendition_a: Some(pl.name.clone()),
                        rendition_b: None,
                        uri_a: None,
                        uri_b: None,
                        message: format!(
                            "rfc8216bis §4.4.3.8: '{}' CAN-SKIP-UNTIL={:.3}s < 6× \
                             TARGETDURATION={:.3}s (MUST be ≥ {:.3}s).",
                            pl.name, csu, pl.target_duration, pl.target_duration * 6.0
                        ),
                        uri_note: Some(format!(
                            "ratio={:.2}×, minimum 6.00×", csu / pl.target_duration
                        )),
                        ..Default::default()
                    });
                }

        // 8. SERVER-CONTROL: PART-HOLD-BACK >= 2× PART-TARGET (MUST), >= 3× (SHOULD)
        if let Some(sc) = &pl.server_control {
            if let (Some(phb), Some(pt)) = (sc.part_hold_back, pl.part_target) {
                if phb < pt * 2.0 - 0.001 {
                    issues.push(Issue {
                        severity: Severity::Error,
                        segment_index: -1,
                        rendition_a: Some(pl.name.clone()),
                        rendition_b: None,
                        uri_a: None,
                        uri_b: None,
                        message: format!(
                            "LL-HLS §4.4.3.8: '{}' PART-HOLD-BACK={:.5}s < 2× PART-TARGET={:.5}s \
                             (MUST be ≥ {:.5}s).",
                            pl.name, phb, pt, pt * 2.0
                        ),
                        uri_note: Some(format!("ratio={:.3}×, MUST be ≥ 2.000×", phb / pt)),
                        ..Default::default()
                    });
                } else if phb < pt * 3.0 - 0.001 {
                    issues.push(Issue {
                        severity: Severity::Warn,
                        segment_index: -1,
                        rendition_a: Some(pl.name.clone()),
                        rendition_b: None,
                        uri_a: None,
                        uri_b: None,
                        message: format!(
                            "LL-HLS §4.4.3.8: '{}' PART-HOLD-BACK={:.5}s < 3× PART-TARGET={:.5}s \
                             (SHOULD be ≥ {:.5}s).",
                            pl.name, phb, pt, pt * 3.0
                        ),
                        uri_note: Some(format!("ratio={:.3}×, SHOULD be ≥ 3.000×", phb / pt)),
                        ..Default::default()
                    });
                }
            }
            // HOLD-BACK >= 3× TARGETDURATION
            if let Some(hb) = sc.hold_back
                && pl.target_duration > 0.0 && hb < pl.target_duration * 3.0 - 0.001 {
                    issues.push(Issue {
                        severity: Severity::Error,
                        segment_index: -1,
                        rendition_a: Some(pl.name.clone()),
                        rendition_b: None,
                        uri_a: None,
                        uri_b: None,
                        message: format!(
                            "LL-HLS §4.4.3.8: '{}' HOLD-BACK={:.3}s < 3× TARGETDURATION={:.3}s \
                             (MUST be ≥ {:.3}s).",
                            pl.name, hb, pl.target_duration, pl.target_duration * 3.0
                        ),
                        uri_note: Some(format!(
                            "ratio={:.2}×, minimum 3.00×", hb / pl.target_duration
                        )),
                        ..Default::default()
                    });
                }
        }
    }

    // ── Cross-rendition: LAST-MSN skew in EXT-X-RENDITION-REPORT ─────────────
    {
        let mut msn_reports: HashMap<String, Vec<i64>> = HashMap::new();
        for pl in playlists {
            if pl.parts.is_empty() { continue; }
            for rr in &pl.rendition_reports {
                if rr.last_msn >= 0 {
                    msn_reports.entry(rr.uri.clone()).or_default().push(rr.last_msn);
                }
            }
        }
        for (uri, msn_list) in &msn_reports {
            if msn_list.len() < 2 { continue; }
            let max_msn = *msn_list.iter().max().unwrap();
            let min_msn = *msn_list.iter().min().unwrap();
            let skew = max_msn - min_msn;
            if skew > 1 {
                let short_uri = uri.rsplit('/').next().unwrap_or(uri.as_str());
                issues.push(Issue::warn(format!(
                    "LL-HLS §4.4.5.4: EXT-X-RENDITION-REPORT LAST-MSN skew of {} segments \
                     for '{}' (reported MSNs: {}–{}). All renditions should report the same \
                     LAST-MSN ±1 segment.",
                    skew, short_uri, min_msn, max_msn
                )));
            }
        }
    }

    // ── Cross-rendition: EXT-X-SERVER-CONTROL must be identical (§6.2.4) ─────
    {
        let ll_pls: Vec<&MediaPlaylist> = playlists.iter()
            .filter(|pl| !pl.parts.is_empty())
            .collect();
        if ll_pls.len() >= 2 {
            type ScKey = (u64, u64, bool, u64);
            let sc_configs: Vec<(&str, ScKey)> = ll_pls.iter().map(|pl| {
                let key = pl.server_control.as_ref().map_or(
                    (0, 0, false, 0),
                    |sc| (
                        sc.hold_back.unwrap_or(0.0) as u64,
                        sc.part_hold_back.unwrap_or(0.0) as u64,
                        sc.can_block_reload,
                        sc.can_skip_until.unwrap_or(0.0) as u64,
                    )
                );
                (pl.name.as_str(), key)
            }).collect();
            let unique: HashSet<ScKey> = sc_configs.iter().map(|(_, k)| *k).collect();
            if unique.len() > 1 {
                let detail: Vec<String> = sc_configs.iter().map(|(name, _)| name.to_string()).collect();
                issues.push(Issue::error(format!(
                    "LL-HLS §6.2.4: EXT-X-SERVER-CONTROL attributes differ across renditions [{}]. \
                     All Media Playlists MUST carry identical SERVER-CONTROL values.",
                    detail.join(", ")
                )));
            }
        }
    }

    produced_by(CheckId::LlHls, issues)
}

/// rfc8216bis §4.4.3.2 — EXT-X-MEDIA-SEQUENCE presence and position.
///
/// Nothing here reads a number out of a segment URI. §4.4.3.2 defines the Media Sequence
/// Number of the first segment as the value of this tag and nothing else; segment file names
/// are free-form, so an MSN inferred from one says nothing about conformance.
pub fn check_media_sequence_continuity(playlists: &[MediaPlaylist]) -> Vec<Issue> {
    let mut issues = Vec::new();
    for pl in playlists {
        if pl.segments.is_empty() { continue; }

        let tag_absent = !pl.raw_content.contains("#EXT-X-MEDIA-SEQUENCE:");
        if tag_absent && !pl.has_endlist && pl.segments.len() > 1 {
            issues.push(Issue::warn(format!(
                "rfc8216bis §4.4.3.2: Live playlist '{}' does not declare \
                 EXT-X-MEDIA-SEQUENCE. For live playlists the tag SHOULD be present \
                 so clients can track the sliding window.",
                pl.name
            )));
        }

        // EXT-X-MEDIA-SEQUENCE MUST appear before the first Media Segment URI.
        //
        // The first segment URI is the first URI line after the first EXTINF, which may be
        // several lines later: BYTERANGE, KEY, MAP and PROGRAM-DATE-TIME tags are all
        // allowed between an EXTINF and the URI it introduces.
        {
            let mut tag_line: Option<usize> = None;
            let mut first_seg_line: Option<usize> = None;
            let mut seen_extinf = false;
            for (i, line) in pl.raw_content.lines().enumerate() {
                let l = line.trim();
                if l.starts_with("#EXT-X-MEDIA-SEQUENCE:") && tag_line.is_none() {
                    tag_line = Some(i);
                }
                if l.starts_with("#EXTINF:") {
                    seen_extinf = true;
                } else if seen_extinf
                    && first_seg_line.is_none()
                    && !l.starts_with('#')
                    && !l.is_empty()
                {
                    first_seg_line = Some(i);
                }
            }
            if let (Some(tl), Some(sl)) = (tag_line, first_seg_line)
                && tl > sl {
                    issues.push(Issue {
                        severity: Severity::Error,
                        segment_index: -1,
                        rendition_a: Some(pl.name.clone()),
                        rendition_b: None,
                        uri_a: None,
                        uri_b: None,
                        message: format!(
                            "rfc8216bis §4.4.3.2: EXT-X-MEDIA-SEQUENCE MUST appear before \
                             the first Media Segment URI in '{}' \
                             (tag at line {}, first segment at line {}).",
                            pl.name, tl + 1, sl + 1
                        ),
                        uri_note: None,
                        ..Default::default()
                    });
                }
        }
    }
    produced_by(CheckId::MediaSequenceTags, issues)
}

/// HLS Interstitials validation (Appendix D)
pub fn check_interstitials(playlists: &[MediaPlaylist]) -> (Vec<Issue>, Vec<Interstitial>) {
    let mut issues = Vec::new();
    let mut interstitials = Vec::new();

    for pl in playlists {
        if pl.raw_content.is_empty() {
            continue;
        }
        // Per rfc8216bis section D.2 a DATERANGE with the same ID as a
        // previously-seen tag is an update; update tags don't need to
        // repeat X-ASSET-URI/X-ASSET-LIST, so we only validate + push
        // the first occurrence of each ID.
        let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for line in pl.raw_content.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("#EXT-X-DATERANGE:") else {
                continue;
            };
            let attrs = super::parser::parse_attributes(rest);
            let class = attrs.get("CLASS").cloned().unwrap_or_default();
            if !class.contains("com.apple.hls.interstitial") {
                continue;
            }
            let dr_id = attrs.get("ID").cloned().unwrap_or_default();
            let is_update = !dr_id.is_empty() && seen_ids.contains(&dr_id);
            if !is_update {
                seen_ids.insert(dr_id.clone());
            }
            if is_update {
                continue;
            }
            let start_date = attrs.get("START-DATE").cloned().unwrap_or_default();
            let asset_uri = attrs.get("X-ASSET-URI").cloned();
            let asset_list = attrs.get("X-ASSET-LIST").cloned();
            let resume_offset = attrs.get("X-RESUME-OFFSET").and_then(|v| v.parse::<f64>().ok());
            let playout_limit = attrs.get("X-PLAYOUT-LIMIT").and_then(|v| v.parse::<f64>().ok());
            // PLANNED-DURATION is present on the OUT tag; X-PLAYOUT-LIMIT may be on the IN tag only
            let planned_duration_s = attrs.get("PLANNED-DURATION").and_then(|v| v.parse::<f64>().ok());
            let snap = attrs.get("X-SNAP").cloned();
            let cue = attrs.get("X-CUE").cloned();
            let timeline_style = attrs.get("X-TIMELINE-STYLE").cloned();

            let mut entry_errors = Vec::new();

            // MUST have ID
            if dr_id.is_empty() {
                entry_errors.push("Missing required ID attribute".to_string());
                issues.push(Issue {
                    severity: Severity::Error,
                    segment_index: -1,
                    rendition_a: Some(pl.name.clone()),
                    rendition_b: None,
                    uri_a: None,
                    uri_b: None,
                    message: format!("Interstitial: DATERANGE in '{}' missing ID (MUST)", pl.name),
                    uri_note: None,
                    ..Default::default()
                });
            }

            // MUST have START-DATE
            if start_date.is_empty() {
                entry_errors.push("Missing required START-DATE".to_string());
                issues.push(Issue {
                    severity: Severity::Error,
                    segment_index: -1,
                    rendition_a: Some(pl.name.clone()),
                    rendition_b: None,
                    uri_a: None,
                    uri_b: None,
                    message: format!("Interstitial: [{}] missing START-DATE in '{}'", dr_id, pl.name),
                    uri_note: None,
                    ..Default::default()
                });
            }

            // MUST have X-ASSET-URI or X-ASSET-LIST
            if asset_uri.is_none() && asset_list.is_none() {
                entry_errors.push("Missing X-ASSET-URI or X-ASSET-LIST (MUST have one)".to_string());
                issues.push(Issue {
                    severity: Severity::Error,
                    segment_index: -1,
                    rendition_a: Some(pl.name.clone()),
                    rendition_b: None,
                    uri_a: None,
                    uri_b: None,
                    message: format!("Interstitial: [{}] missing X-ASSET-URI/X-ASSET-LIST in '{}'", dr_id, pl.name),
                    uri_note: None,
                    ..Default::default()
                });
            }

            // MUST NOT have both
            if asset_uri.is_some() && asset_list.is_some() {
                entry_errors.push("Has both X-ASSET-URI and X-ASSET-LIST (MUST have only one)".to_string());
                issues.push(Issue {
                    severity: Severity::Error,
                    segment_index: -1,
                    rendition_a: Some(pl.name.clone()),
                    rendition_b: None,
                    uri_a: None,
                    uri_b: None,
                    message: format!("Interstitial: [{}] has both X-ASSET-URI and X-ASSET-LIST in '{}' (MUST NOT)", dr_id, pl.name),
                    uri_note: None,
                    ..Default::default()
                });
            }

            // Validate X-SNAP — §D.2: enumerated-string-list, values IN and OUT only
            if let Some(ref s) = snap {
                for part in s.split(',') {
                    let p = part.trim();
                    if p != "IN" && p != "OUT" {
                        entry_errors.push(format!("X-SNAP value '{}' invalid (must be IN or OUT)", p));
                        issues.push(Issue {
                            severity: Severity::Warn,
                            segment_index: -1,
                            rendition_a: Some(pl.name.clone()),
                            rendition_b: None,
                            uri_a: None,
                            uri_b: None,
                            message: format!(
                                "Interstitial: [{}] X-SNAP='{}' invalid in '{}' \
                                 (rfc8216bis §D.2: values must be OUT and/or IN).",
                                dr_id, p, pl.name
                            ),
                            uri_note: None,
                            ..Default::default()
                        });
                    }
                }
            }

            // Validate X-RESTRICT — §D.2: enumerated-string-list, values SKIP and JUMP only
            if let Some(ref restrict) = attrs.get("X-RESTRICT").cloned() {
                for part in restrict.split(',') {
                    let p = part.trim();
                    if p != "SKIP" && p != "JUMP" {
                        entry_errors.push(format!("X-RESTRICT value '{}' invalid (must be SKIP or JUMP)", p));
                        issues.push(Issue {
                            severity: Severity::Warn,
                            segment_index: -1,
                            rendition_a: Some(pl.name.clone()),
                            rendition_b: None,
                            uri_a: None,
                            uri_b: None,
                            message: format!(
                                "Interstitial: [{}] X-RESTRICT='{}' invalid in '{}' \
                                 (rfc8216bis §D.2: values must be SKIP and/or JUMP).",
                                dr_id, p, pl.name
                            ),
                            uri_note: None,
                            ..Default::default()
                        });
                    }
                }
            }

            // Validate X-CONTENT-MAY-VARY — §D.2: valid values "YES" and "NO"
            if let Some(ref cmv) = attrs.get("X-CONTENT-MAY-VARY").cloned()
                && cmv != "YES" && cmv != "NO" {
                    entry_errors.push(format!("X-CONTENT-MAY-VARY='{}' invalid (must be YES or NO)", cmv));
                    issues.push(Issue {
                        severity: Severity::Warn,
                        segment_index: -1,
                        rendition_a: Some(pl.name.clone()),
                        rendition_b: None,
                        uri_a: None,
                        uri_b: None,
                        message: format!(
                            "Interstitial: [{}] X-CONTENT-MAY-VARY='{}' invalid in '{}' \
                             (rfc8216bis §D.2: must be \"YES\" or \"NO\").",
                            dr_id, cmv, pl.name
                        ),
                        uri_note: None,
                        ..Default::default()
                    });
                }

            // Validate X-TIMELINE-OCCUPIES — §D.2: valid values "POINT" and "RANGE"
            if let Some(ref to) = attrs.get("X-TIMELINE-OCCUPIES").cloned()
                && to != "POINT" && to != "RANGE" {
                    entry_errors.push(format!("X-TIMELINE-OCCUPIES='{}' invalid (must be POINT or RANGE)", to));
                    issues.push(Issue {
                        severity: Severity::Warn,
                        segment_index: -1,
                        rendition_a: Some(pl.name.clone()),
                        rendition_b: None,
                        uri_a: None,
                        uri_b: None,
                        message: format!(
                            "Interstitial: [{}] X-TIMELINE-OCCUPIES='{}' invalid in '{}' \
                             (rfc8216bis §D.2: must be \"POINT\" or \"RANGE\").",
                            dr_id, to, pl.name
                        ),
                        uri_note: None,
                        ..Default::default()
                    });
                }

            // Validate X-TIMELINE-STYLE — §D.2: valid values "HIGHLIGHT" and "PRIMARY"
            if let Some(ref ts) = attrs.get("X-TIMELINE-STYLE").cloned()
                && ts != "HIGHLIGHT" && ts != "PRIMARY" {
                    entry_errors.push(format!("X-TIMELINE-STYLE='{}' invalid (must be HIGHLIGHT or PRIMARY)", ts));
                    issues.push(Issue {
                        severity: Severity::Warn,
                        segment_index: -1,
                        rendition_a: Some(pl.name.clone()),
                        rendition_b: None,
                        uri_a: None,
                        uri_b: None,
                        message: format!(
                            "Interstitial: [{}] X-TIMELINE-STYLE='{}' invalid in '{}' \
                             (rfc8216bis §D.2: must be \"HIGHLIGHT\" or \"PRIMARY\").",
                            dr_id, ts, pl.name
                        ),
                        uri_note: None,
                        ..Default::default()
                    });
                }

            // Validate X-SKIP-CONTROL-LABEL-ID — §D.3: characters must be [a-z][A-Z]'-''_' only
            if let Some(ref label_id) = attrs.get("X-SKIP-CONTROL-LABEL-ID").cloned() {
                let invalid_chars: Vec<char> = label_id.chars()
                    .filter(|&c| !c.is_ascii_alphabetic() && c != '-' && c != '_')
                    .collect();
                if !invalid_chars.is_empty() {
                    entry_errors.push(format!(
                        "X-SKIP-CONTROL-LABEL-ID='{}' contains invalid chars {:?}",
                        label_id, invalid_chars
                    ));
                    issues.push(Issue {
                        severity: Severity::Error,
                        segment_index: -1,
                        rendition_a: Some(pl.name.clone()),
                        rendition_b: None,
                        uri_a: None,
                        uri_b: None,
                        message: format!(
                            "Interstitial: [{}] X-SKIP-CONTROL-LABEL-ID='{}' contains \
                             invalid characters {:?} in '{}' \
                             (rfc8216bis §D.3: MUST be [a-z][A-Z]'-''_' only).",
                            dr_id, label_id, invalid_chars, pl.name
                        ),
                        uri_note: None,
                        ..Default::default()
                    });
                }
            }

            interstitials.push(Interstitial {
                rendition: pl.name.clone(),
                id: dr_id,
                start_date,
                asset_uri,
                asset_list,
                resume_offset,
                playout_limit,
                planned_duration_s,
                snap,
                cue,
                timeline_style,
                errors: entry_errors,
                start_offset_s: None,
                content_duration_s: 0.0,
                rendition_url: pl.url.clone(),
                definitions: pl.definitions.clone(),
            });
        }
    }

    // ── Second pass: merge IN-tag attributes (X-PLAYOUT-LIMIT, X-RESUME-OFFSET) ──────────────
    // The OUT DATERANGE carries X-ASSET-LIST, PLANNED-DURATION, CLASS, etc.
    // The paired IN DATERANGE (same ID, no CLASS) carries X-PLAYOUT-LIMIT and X-RESUME-OFFSET.
    // Build a map from ID → index of the FIRST occurrence in `interstitials`.
    let mut id_to_idx: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (idx, it) in interstitials.iter().enumerate() {
        id_to_idx.entry(it.id.clone()).or_insert(idx);
    }
    for pl in playlists {
        for line in pl.raw_content.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("#EXT-X-DATERANGE:") else { continue; };
            let attrs = super::parser::parse_attributes(rest);
            // Skip OUT tags (already processed above) — only want IN tags (no CLASS)
            if attrs.get("CLASS").is_some_and(|c| c.contains("com.apple.hls.interstitial")) {
                continue;
            }
            let id = attrs.get("ID").cloned().unwrap_or_default();
            if let Some(&idx) = id_to_idx.get(&id) {
                let it = &mut interstitials[idx];
                if it.playout_limit.is_none() {
                    it.playout_limit = attrs.get("X-PLAYOUT-LIMIT").and_then(|v| v.parse::<f64>().ok());
                }
                if it.resume_offset.is_none() {
                    it.resume_offset = attrs.get("X-RESUME-OFFSET").and_then(|v| v.parse::<f64>().ok());
                }
            }
        }
    }

    (produced_by(CheckId::Interstitials, issues), interstitials)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn make_playlist(name: &str, content: &str) -> MediaPlaylist {
        let mut pl = MediaPlaylist::new(name.to_string(), format!("https://cdn.example.com/{name}.m3u8"));
        pl.raw_content = content.to_string();
        pl
    }

    fn make_segment(uri: &str, duration: f64) -> Segment {
        Segment {
            uri: uri.to_string(),
            duration,
            title: None,
            pdt: None,
            discontinuity: false,
            byterange: None,
            is_ad: false,
            map_uri: None,
        }
    }

    fn make_segment_with_pdt(uri: &str, duration: f64, pdt: f64) -> Segment {
        Segment { pdt: Some(pdt), ..make_segment(uri, duration) }
    }

    /// Read a media playlist the way the validator does, so tests exercise the parser's own
    /// view of segments, PDTs and tag order rather than a hand-built one.
    fn parse_playlist(name: &str, content: &str) -> MediaPlaylist {
        let url = format!("https://cdn.example.com/{name}.m3u8");
        let mut pl = MediaPlaylist::new(name.to_string(), url.clone());
        super::super::parser::parse_media_playlist(&url, content, &mut pl);
        pl
    }

    fn parse_master(content: &str) -> MasterPlaylist {
        super::super::parser::parse_master_playlist("https://cdn.example.com/master.m3u8", content)
    }

    fn errors(issues: &[Issue]) -> Vec<&Issue> {
        issues.iter().filter(|i| i.severity == Severity::Error).collect()
    }

    // ── check_extm3u_header ───────────────────────────────────────────────────

    #[test]
    fn extm3u_header_passes_when_first_line_is_extm3u() {
        let pl = make_playlist("v", "#EXTM3U\n#EXT-X-TARGETDURATION:6\n");
        let issues = check_extm3u_header(&[pl]);
        assert!(issues.is_empty(), "expected no issues, got: {:?}", issues);
    }

    #[test]
    fn extm3u_header_errors_when_first_line_is_missing() {
        let pl = make_playlist("v", "#EXT-X-TARGETDURATION:6\n#EXTM3U\n");
        let issues = check_extm3u_header(&[pl]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
    }

    // ── check_target_duration_compliance ─────────────────────────────────────

    #[test]
    fn target_duration_missing_produces_error() {
        let pl = make_playlist("v", "#EXTM3U\n");
        let issues = check_target_duration_compliance(&[pl]);
        assert!(issues.iter().any(|i| i.severity == Severity::Error && i.message.contains("missing or zero")));
    }

    #[test]
    fn segment_within_target_duration_passes() {
        let mut pl = make_playlist("v", "#EXTM3U\n#EXT-X-TARGETDURATION:6\n");
        pl.target_duration = 6.0;
        pl.segments = vec![make_segment("seg0.mp4", 5.9)];
        let issues = check_target_duration_compliance(&[pl]);
        assert!(issues.is_empty(), "expected no issues");
    }

    #[test]
    fn segment_rounding_to_target_passes() {
        // 5.5 rounds to 6 which equals TARGETDURATION:6 — should pass
        let mut pl = make_playlist("v", "#EXTM3U\n#EXT-X-TARGETDURATION:6\n");
        pl.target_duration = 6.0;
        pl.segments = vec![make_segment("seg0.mp4", 5.5)];
        let issues = check_target_duration_compliance(&[pl]);
        assert!(issues.is_empty(), "5.5 rounds to 6 = TARGETDURATION, must pass");
    }

    #[test]
    fn segment_exceeding_target_duration_errors() {
        // 7.807 rounds to 8 > 6 → ERROR
        let mut pl = make_playlist("v", "#EXTM3U\n#EXT-X-TARGETDURATION:6\n");
        pl.target_duration = 6.0;
        pl.segments = vec![make_segment("seg-bad.mp4", 7.807)];
        let issues = check_target_duration_compliance(&[pl]);
        assert!(issues.iter().any(|i| i.severity == Severity::Error && i.message.contains("exceeds TARGETDURATION")));
    }

    #[test]
    fn targetduration_much_larger_than_max_segment_warns() {
        // TARGETDURATION=10, max segment=3.9 → WARN because 10 > 3+1=4
        let mut pl = make_playlist("v", "#EXTM3U\n#EXT-X-TARGETDURATION:10\n");
        pl.target_duration = 10.0;
        pl.segments = vec![make_segment("seg0.mp4", 3.9), make_segment("seg1.mp4", 3.9)];
        let issues = check_target_duration_compliance(&[pl]);
        assert!(issues.iter().any(|i| i.severity == Severity::Warn && i.message.contains("more than 1s above")));
    }

    // ── check_pdt_coverage ────────────────────────────────────────────────────

    #[test]
    fn pdt_coverage_all_present_no_issues() {
        let mut pl = make_playlist("v", "#EXTM3U\n");
        let base = 1_700_000_000.0_f64;
        pl.segments = vec![
            make_segment_with_pdt("s0.mp4", 4.0, base),
            make_segment_with_pdt("s1.mp4", 4.0, base + 4.0),
        ];
        let issues = check_pdt_coverage(&[pl]);
        assert!(issues.is_empty());
    }

    #[test]
    fn pdt_coverage_partial_warns() {
        let mut pl = make_playlist("v", "#EXTM3U\n");
        pl.segments = vec![
            make_segment_with_pdt("s0.mp4", 4.0, 1_700_000_000.0),
            make_segment("s1.mp4", 4.0),  // no PDT
        ];
        let issues = check_pdt_coverage(&[pl]);
        assert!(issues.iter().any(|i| i.severity == Severity::Warn && i.message.contains("partial PDT")));
    }

    #[test]
    fn pdt_coverage_none_no_issues() {
        let mut pl = make_playlist("v", "#EXTM3U\n");
        pl.segments = vec![make_segment("s0.mp4", 4.0), make_segment("s1.mp4", 4.0)];
        let issues = check_pdt_coverage(&[pl]);
        assert!(issues.is_empty());
    }

    // ── check_media_sequence_duplicate_tags ───────────────────────────────────

    #[test]
    fn duplicate_targetduration_errors() {
        let content = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-TARGETDURATION:4\n";
        let pl = make_playlist("v", content);
        let issues = check_media_sequence_duplicate_tags(&[pl]);
        assert!(issues.iter().any(|i| i.severity == Severity::Error && i.message.contains("EXT-X-TARGETDURATION")));
    }

    #[test]
    fn single_targetduration_passes() {
        let content = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n";
        let pl = make_playlist("v", content);
        let issues = check_media_sequence_duplicate_tags(&[pl]);
        assert!(issues.is_empty());
    }

    // ── check_live_playlist_min_segments ─────────────────────────────────────

    #[test]
    fn live_playlist_with_two_segments_errors() {
        let mut pl = make_playlist("v", "#EXTM3U\n");
        pl.has_endlist = false;
        pl.segments = vec![make_segment("s0.mp4", 4.0), make_segment("s1.mp4", 4.0)];
        let issues = check_live_playlist_min_segments(&[pl]);
        assert!(issues.iter().any(|i| i.severity == Severity::Error && i.message.contains("at least 3 segments")));
    }

    #[test]
    fn vod_playlist_with_two_segments_passes_min_segment_check() {
        let mut pl = make_playlist("v", "#EXTM3U\n#EXT-X-ENDLIST\n");
        pl.has_endlist = true;
        pl.segments = vec![make_segment("s0.mp4", 4.0), make_segment("s1.mp4", 4.0)];
        let issues = check_live_playlist_min_segments(&[pl]);
        assert!(issues.is_empty(), "VOD playlist must not trigger live-segment count check");
    }

    // ── check_playlist_type_endlist ───────────────────────────────────────────

    #[test]
    fn vod_without_endlist_errors() {
        let mut pl = make_playlist("v", "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:VOD\n");
        pl.playlist_type = Some("VOD".to_string());
        pl.has_endlist = false;
        let issues = check_playlist_type_endlist(&[pl]);
        assert!(issues.iter().any(|i| i.severity == Severity::Error && i.message.contains("PLAYLIST-TYPE:VOD")));
    }

    #[test]
    fn vod_with_endlist_passes() {
        let mut pl = make_playlist("v", "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-ENDLIST\n");
        pl.playlist_type = Some("VOD".to_string());
        pl.has_endlist = true;
        let issues = check_playlist_type_endlist(&[pl]);
        assert!(issues.is_empty());
    }

    #[test]
    fn event_without_endlist_passes() {
        // EVENT playlist without ENDLIST is valid during a live event
        let mut pl = make_playlist("v", "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:EVENT\n");
        pl.playlist_type = Some("EVENT".to_string());
        pl.has_endlist = false;
        let issues = check_playlist_type_endlist(&[pl]);
        assert!(issues.is_empty());
    }

    // ── check identity ────────────────────────────────────────────────────────

    /// Every finding the checks can produce has to name the check that produced it, because
    /// the report groups findings by that name and anything unnamed lands in a catch-all row.
    #[test]
    fn every_check_names_itself_on_every_finding() {
        let broken = parse_playlist(
            "broken",
            "#EXT-X-TARGETDURATION:6\n\
             #EXT-X-VERSION:1\n\
             #EXT-X-TARGETDURATION:6\n\
             #EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n\
             #EXT-X-MAP:URI=\"init.mp4\"\n\
             #EXT-X-DISCONTINUITY-SEQUENCE:2\n\
             #EXT-X-PART-INF:PART-TARGET=1.0\n\
             #EXT-X-SERVER-CONTROL:CAN-SKIP-UNTIL=6.0,HOLD-BACK=1.0\n\
             #EXT-X-DATERANGE:ID=\"ad-1\",CLASS=\"com.apple.hls.interstitial\",\
             START-DATE=\"2024-01-15T12:00:00Z\"\n\
             #EXT-X-PART:DURATION=2.0,URI=\"p0.m4s\"\n\
             #EXTINF:7.807,\n\
             #EXT-X-PROGRAM-DATE-TIME:2024-01-15T12:00:00Z\n\
             s0.m4s\n\
             #EXT-X-DISCONTINUITY\n\
             #EXTINF:4.5,\ns1.m4s\n\
             orphan.m4s\n\
             #EXT-X-MEDIA-SEQUENCE:10\n",
        );
        let other = parse_playlist(
            "other",
            "#EXTM3U\n#EXT-X-VERSION:9\n#EXT-X-TARGETDURATION:4\n\
             #EXT-X-MEDIA-SEQUENCE:10\n#EXT-X-DISCONTINUITY-SEQUENCE:5\n\
             #EXT-X-KEY:METHOD=NONE\n#EXT-X-PLAYLIST-TYPE:VOD\n\
             #EXTINF:4.0,\ns0.m4s\n#EXTINF:4.0,\ns1.m4s\n",
        );
        let master = parse_master(
            "#EXT-X-VERSION:6\n#EXT-X-VERSION:7\n\
             #EXT-X-DEFINE:NAME=\"h\",VALUE=\"https://cdn\"\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a1\",NAME=\"en\",URI=\"a.m3u8\"\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a2\",NAME=\"fr\",URI=\"b.m3u8\"\n\
             #EXT-X-STREAM-INF:AUDIO=\"missing\",CODECS=\"avc1.64001f\"\nv.m3u8\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1000,CODECS=\"hvc1.2.4.L153\"\nv.m3u8\n",
        );

        let playlists = [broken, other];
        let mut findings: Vec<Issue> = Vec::new();
        for pl in &playlists {
            findings.extend(pl.parse_issues.iter().cloned());
        }
        findings.extend(check_extm3u_header(&playlists));
        findings.extend(check_target_duration_compliance(&playlists));
        findings.extend(check_pdt_coverage(&playlists));
        findings.extend(check_media_sequence_duplicate_tags(&playlists));
        findings.extend(check_version_compatibility(&playlists));
        findings.extend(check_live_playlist_min_segments(&playlists));
        findings.extend(check_targetduration_consistency(&playlists));
        findings.extend(check_playlist_type_endlist(&playlists));
        findings.extend(check_encryption_consistency(&playlists));
        findings.extend(check_discontinuity_sequence(&playlists));
        findings.extend(check_segment_count(&playlists));
        findings.extend(check_duration_drift(&playlists, 100.0));
        findings.extend(check_pdt_alignment(&playlists, 100.0));
        findings.extend(check_cumulative_drift(&playlists, 100.0));
        findings.extend(check_ll_hls_compliance(&playlists));
        findings.extend(check_media_sequence_continuity(&playlists));
        findings.extend(check_interstitials(&playlists).0);
        findings.extend(check_master_structure(&master));
        findings.extend(check_stream_inf_consistency(&master));
        findings.extend(check_bandwidth_required(&master));
        findings.extend(check_media_group_membership(&master));
        findings.extend(check_rendition_group_references(&master));

        let unnamed: Vec<&str> = findings.iter()
            .filter(|i| i.check_id == CheckId::Unassigned)
            .map(|i| i.message.as_str())
            .collect();
        assert!(unnamed.is_empty(), "findings that name no check: {unnamed:#?}");

        // Every check the fixture reaches. The ids deliberately not listed are PdtAlignment
        // and SegmentCount, which need a second fully PDT-tagged / VOD-with-ENDLIST rendition
        // this fixture does not have, and DeltaUpdates and PlaylistFetch, which are only
        // produced by the fetching code in the parent module (covered by its own tests).
        let named: HashSet<CheckId> = findings.iter().map(|i| i.check_id).collect();
        let expected = [
            CheckId::BandwidthRequired,
            CheckId::CumulativeDrift,
            CheckId::DiscontinuitySequence,
            CheckId::DurationDrift,
            CheckId::EncryptionConsistency,
            CheckId::ExtM3uHeader,
            CheckId::Interstitials,
            CheckId::LlHls,
            CheckId::LivePlaylistWindow,
            CheckId::MediaGroupMembership,
            CheckId::MediaSequenceTags,
            CheckId::PdtCoverage,
            CheckId::PlaylistTypeEndlist,
            CheckId::RenditionGroupReferences,
            CheckId::SegmentStructure,
            CheckId::SingletonTags,
            CheckId::StreamInfConsistency,
            CheckId::TargetDurationCompliance,
            CheckId::TargetDurationConsistency,
            CheckId::VersionCompatibility,
        ];
        let missing: Vec<CheckId> = expected.iter()
            .filter(|id| !named.contains(id))
            .copied()
            .collect();
        assert!(
            missing.is_empty(),
            "these checks stopped reporting, or stopped naming themselves: {missing:?}"
        );
    }

    // ── check_media_sequence_continuity ───────────────────────────────────────

    #[test]
    fn segment_uri_numbers_are_not_read_as_media_sequence_numbers() {
        // §4.4.3.2 defines the MSN of the first segment as EXT-X-MEDIA-SEQUENCE and nothing
        // else. Numeric segment file names that do not line up with it are not a violation.
        let pl = parse_playlist(
            "v",
            "#EXTM3U\n\
             #EXT-X-TARGETDURATION:4\n\
             #EXT-X-MEDIA-SEQUENCE:10\n\
             #EXTINF:4.0,\n151674692.m4v\n\
             #EXTINF:4.0,\n151674693.m4v\n\
             #EXTINF:4.0,\n20260715T225158-151674694-03-ts.m4v\n",
        );
        let issues = check_media_sequence_continuity(&[pl]);
        assert!(issues.is_empty(), "URI-derived MSNs must not be reported: {issues:?}");
    }

    #[test]
    fn media_sequence_without_the_tag_warns_on_live_only() {
        let live = parse_playlist(
            "live",
            "#EXTM3U\n#EXT-X-TARGETDURATION:4\n\
             #EXTINF:4.0,\ns0.m4s\n#EXTINF:4.0,\ns1.m4s\n",
        );
        let issues = check_media_sequence_continuity(&[live]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Warn, "the tag is a SHOULD for live playlists");

        let vod = parse_playlist(
            "vod",
            "#EXTM3U\n#EXT-X-TARGETDURATION:4\n\
             #EXTINF:4.0,\ns0.m4s\n#EXTINF:4.0,\ns1.m4s\n#EXT-X-ENDLIST\n",
        );
        assert!(check_media_sequence_continuity(&[vod]).is_empty());
    }

    #[test]
    fn media_sequence_after_the_first_segment_is_found_across_intervening_tags() {
        // The first segment URI is three lines below its EXTINF; the old scan only looked at
        // the line immediately after an EXTINF and so found no segment at all.
        let pl = parse_playlist(
            "v",
            "#EXTM3U\n\
             #EXT-X-TARGETDURATION:4\n\
             #EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n\
             #EXTINF:4.0,\n\
             #EXT-X-PROGRAM-DATE-TIME:2024-01-15T12:00:00Z\n\
             #EXT-X-BYTERANGE:1000@0\n\
             s0.m4s\n\
             #EXT-X-MEDIA-SEQUENCE:10\n\
             #EXTINF:4.0,\ns1.m4s\n",
        );
        let issues = check_media_sequence_continuity(&[pl]);
        assert!(
            errors(&issues).iter().any(|i| i.message.contains("MUST appear before")),
            "expected a tag-order error: {issues:?}"
        );
    }

    #[test]
    fn media_sequence_before_the_first_segment_passes_with_intervening_tags() {
        let pl = parse_playlist(
            "v",
            "#EXTM3U\n\
             #EXT-X-TARGETDURATION:4\n\
             #EXT-X-MEDIA-SEQUENCE:10\n\
             #EXTINF:4.0,\n\
             #EXT-X-BYTERANGE:1000@0\n\
             s0.m4s\n",
        );
        assert!(check_media_sequence_continuity(&[pl]).is_empty());
    }

    // ── check_targetduration_consistency ─────────────────────────────────────

    #[test]
    fn targetduration_mismatch_across_renditions_errors() {
        let a = parse_playlist("v0", "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\ns0.m4s\n");
        let b = parse_playlist("v1", "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4.0,\ns0.m4s\n");
        let issues = check_targetduration_consistency(&[a, b]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error, "§6.2.4 states this as a MUST");
    }

    #[test]
    fn vod_iframe_playlist_may_declare_its_own_targetduration() {
        let video = parse_playlist(
            "v0",
            "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-PLAYLIST-TYPE:VOD\n\
             #EXTINF:6.0,\ns0.m4s\n#EXT-X-ENDLIST\n",
        );
        let trick = parse_playlist(
            "iframe",
            "#EXTM3U\n#EXT-X-TARGETDURATION:60\n#EXT-X-PLAYLIST-TYPE:VOD\n\
             #EXT-X-I-FRAMES-ONLY\n#EXTINF:60.0,\ni0.m4s\n#EXT-X-ENDLIST\n",
        );
        let issues = check_targetduration_consistency(&[video, trick]);
        assert!(issues.is_empty(), "§6.2.4 exempts VOD I-frame playlists: {issues:?}");
    }

    // ── check_discontinuity_sequence ──────────────────────────────────────────

    #[test]
    fn discontinuity_sequence_mismatch_errors() {
        let a = parse_playlist(
            "v0",
            "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-DISCONTINUITY-SEQUENCE:2\n\
             #EXTINF:4.0,\ns0.m4s\n",
        );
        let b = parse_playlist(
            "v1",
            "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-DISCONTINUITY-SEQUENCE:5\n\
             #EXTINF:4.0,\ns0.m4s\n",
        );
        let issues = check_discontinuity_sequence(&[a, b]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
        assert_eq!(issues[0].check_id, CheckId::DiscontinuitySequence);
    }

    // ── check_encryption_consistency ──────────────────────────────────────────

    #[test]
    fn a_clear_rendition_alongside_an_encrypted_one_warns_but_does_not_fail() {
        let encrypted = parse_playlist(
            "v0",
            "#EXTM3U\n#EXT-X-TARGETDURATION:4\n\
             #EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n#EXTINF:4.0,\ns0.m4s\n",
        );
        let clear = parse_playlist(
            "iframe",
            "#EXTM3U\n#EXT-X-TARGETDURATION:4\n\
             #EXT-X-KEY:METHOD=NONE\n#EXTINF:4.0,\ni0.m4s\n",
        );
        let issues = check_encryption_consistency(&[encrypted, clear]);
        assert!(
            errors(&issues).is_empty(),
            "a mixed clear/encrypted presentation must not fail the run: {issues:?}"
        );
        assert!(issues.iter().any(|i| i.severity == Severity::Warn), "but it is worth a warning");
    }

    // ── check_version_compatibility ───────────────────────────────────────────

    #[test]
    fn map_without_iframes_only_requires_version_6() {
        let v5 = parse_playlist(
            "v",
            "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:4\n\
             #EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:4.0,\ns0.m4s\n",
        );
        let issues = check_version_compatibility(&[v5]);
        assert!(
            errors(&issues).iter().any(|i| i.message.contains("VERSION >= 6")),
            "EXT-X-MAP outside an I-frame playlist needs v6: {issues:?}"
        );

        let v6 = parse_playlist(
            "v",
            "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:4\n\
             #EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:4.0,\ns0.m4s\n",
        );
        assert!(check_version_compatibility(&[v6]).is_empty());
    }

    #[test]
    fn map_in_an_iframes_only_playlist_is_allowed_at_version_5() {
        let pl = parse_playlist(
            "iframe",
            "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:60\n#EXT-X-I-FRAMES-ONLY\n\
             #EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:60.0,\ni0.m4s\n",
        );
        let issues = check_version_compatibility(&[pl]);
        assert!(issues.is_empty(), "§8 allows MAP at v5 with I-FRAMES-ONLY: {issues:?}");
    }

    // ── check_ll_hls_compliance ───────────────────────────────────────────────

    fn ll_playlist(server_control: &str, part_inf: bool, independent_first_part: bool) -> MediaPlaylist {
        let first_part = if independent_first_part { ",INDEPENDENT=YES" } else { "" };
        let content = format!(
            "#EXTM3U\n#EXT-X-VERSION:9\n#EXT-X-TARGETDURATION:4\n\
             #EXT-X-MEDIA-SEQUENCE:1\n\
             {server_control}\n\
             {}\
             #EXT-X-PART:DURATION=1.0,URI=\"p0.m4s\"{first_part}\n\
             #EXT-X-PART:DURATION=1.0,URI=\"p1.m4s\"\n\
             #EXTINF:4.0,\ns0.m4s\n",
            if part_inf { "#EXT-X-PART-INF:PART-TARGET=1.0\n" } else { "" },
        );
        parse_playlist("v", &content)
    }

    #[test]
    fn a_dependent_first_part_is_not_an_error() {
        // INDEPENDENT is an OPTIONAL attribute; §4.4.4.9 only recommends it on the first part.
        let pl = ll_playlist(
            "#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=3.0,HOLD-BACK=12.0",
            true,
            false,
        );
        let issues = check_ll_hls_compliance(&[pl]);
        assert!(
            !issues.iter().any(|i| i.message.contains("INDEPENDENT")),
            "a dependent first part must not be reported: {issues:?}"
        );
    }

    #[test]
    fn a_missing_can_block_reload_is_not_an_error() {
        let pl = ll_playlist(
            "#EXT-X-SERVER-CONTROL:PART-HOLD-BACK=3.0,HOLD-BACK=12.0",
            true,
            true,
        );
        let issues = check_ll_hls_compliance(&[pl]);
        assert!(
            !issues.iter().any(|i| i.message.contains("CAN-BLOCK-RELOAD")),
            "CAN-BLOCK-RELOAD is how a server advertises blocking reload, not a requirement \
             on the playlist: {issues:?}"
        );
    }

    #[test]
    fn part_inf_without_part_hold_back_errors() {
        let pl = ll_playlist(
            "#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,HOLD-BACK=12.0",
            true,
            true,
        );
        let issues = check_ll_hls_compliance(&[pl]);
        assert!(
            errors(&issues).iter().any(|i| i.message.contains("PART-HOLD-BACK")
                && i.message.contains("REQUIRED")),
            "§4.4.3.8 makes PART-HOLD-BACK REQUIRED alongside EXT-X-PART-INF: {issues:?}"
        );
    }

    #[test]
    fn part_hold_back_present_satisfies_the_requirement() {
        let pl = ll_playlist(
            "#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=3.0,HOLD-BACK=12.0",
            true,
            true,
        );
        let issues = check_ll_hls_compliance(&[pl]);
        assert!(
            !issues.iter().any(|i| i.message.contains("no PART-HOLD-BACK")),
            "unexpected PART-HOLD-BACK finding: {issues:?}"
        );
    }

    // ── check_stream_inf_consistency ──────────────────────────────────────────

    #[test]
    fn same_uri_and_groups_with_different_codecs_warns() {
        let master = parse_master(
            "#EXTM3U\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"en\",URI=\"a.m3u8\"\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1000,AUDIO=\"aac\",CODECS=\"avc1.64001f,mp4a.40.2\"\nv.m3u8\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1000,AUDIO=\"aac\",CODECS=\"avc1.64001f,mp4a.40.5\"\nv.m3u8\n",
        );
        let issues = check_stream_inf_consistency(&master);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].severity, Severity::Warn, "a CODECS mismatch must not fail the run");
        assert!(issues[0].message.contains("§4.4.6.2"), "citation: {}", issues[0].message);
    }

    #[test]
    fn same_uri_with_different_subtitle_groups_is_a_different_variant_stream() {
        // Two Variant Streams sharing a video URI but pairing it with different SUBTITLES
        // groups describe different presentations; their CODECS and BANDWIDTH may differ.
        let master = parse_master(
            "#EXTM3U\n\
             #EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs-en\",NAME=\"en\",URI=\"en.m3u8\"\n\
             #EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs-fr\",NAME=\"fr\",URI=\"fr.m3u8\"\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1000,SUBTITLES=\"subs-en\",CODECS=\"avc1.64001f\"\nv.m3u8\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1200,SUBTITLES=\"subs-fr\",CODECS=\"avc1.64001f,mp4a.40.2\"\nv.m3u8\n",
        );
        let issues = check_stream_inf_consistency(&master);
        assert!(issues.is_empty(), "expected no findings: {issues:?}");
    }

    #[test]
    fn same_uri_with_different_video_codecs_errors() {
        let master = parse_master(
            "#EXTM3U\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"en\",URI=\"a.m3u8\"\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"ec3\",NAME=\"en\",URI=\"a2.m3u8\"\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1000,AUDIO=\"aac\",CODECS=\"avc1.64001f,mp4a.40.2\"\nv.m3u8\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1400,AUDIO=\"ec3\",CODECS=\"hvc1.2.4.L153,ec-3\"\nv.m3u8\n",
        );
        let issues = check_stream_inf_consistency(&master);
        let errs = errors(&issues);
        assert_eq!(errs.len(), 1, "{issues:?}");
        assert!(errs[0].message.contains("§6.2.4"), "citation: {}", errs[0].message);
    }

    #[test]
    fn same_uri_with_different_video_groups_is_a_different_variant_stream() {
        // VIDEO pairs a Variant Stream with an alternative video rendition group the same way
        // AUDIO does, so it is part of what makes two STREAM-INF entries the same stream.
        let master = parse_master(
            "#EXTM3U\n\
             #EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"cam-main\",NAME=\"main\",URI=\"m.m3u8\"\n\
             #EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"cam-alt\",NAME=\"alt\",URI=\"c.m3u8\"\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1000,VIDEO=\"cam-main\",CODECS=\"avc1.64001f\"\nv.m3u8\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1200,VIDEO=\"cam-alt\",CODECS=\"avc1.64001f,mp4a.40.2\"\nv.m3u8\n",
        );
        let issues = check_stream_inf_consistency(&master);
        assert!(issues.is_empty(), "expected no findings: {issues:?}");
    }

    #[test]
    fn same_uri_and_groups_with_different_bandwidth_warns() {
        let master = parse_master(
            "#EXTM3U\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1000,CODECS=\"avc1.64001f\"\nv.m3u8\n\
             #EXT-X-STREAM-INF:BANDWIDTH=2000,CODECS=\"avc1.64001f\"\nv.m3u8\n",
        );
        let issues = check_stream_inf_consistency(&master);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].severity, Severity::Warn);
        assert!(issues[0].message.contains("BANDWIDTH"));
    }

    // ── check_master_structure ────────────────────────────────────────────────

    #[test]
    fn master_without_extm3u_errors() {
        let master = parse_master("#EXT-X-STREAM-INF:BANDWIDTH=1000\nv.m3u8\n");
        let issues = check_master_structure(&master);
        assert!(
            errors(&issues).iter().any(|i| i.check_id == CheckId::ExtM3uHeader),
            "{issues:?}"
        );
    }

    #[test]
    fn master_repeating_a_singleton_tag_errors() {
        let master = parse_master(
            "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-VERSION:7\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1000\nv.m3u8\n",
        );
        let issues = check_master_structure(&master);
        assert!(
            errors(&issues).iter().any(|i| i.check_id == CheckId::SingletonTags
                && i.message.contains("EXT-X-VERSION")),
            "{issues:?}"
        );
    }

    #[test]
    fn master_using_variable_substitution_below_version_8_errors() {
        let master = parse_master(
            "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-DEFINE:NAME=\"host\",VALUE=\"https://cdn\"\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1000\n{$host}/v.m3u8\n",
        );
        let issues = check_master_structure(&master);
        assert!(
            errors(&issues).iter().any(|i| i.check_id == CheckId::VersionCompatibility
                && i.message.contains("VERSION >= 8")),
            "{issues:?}"
        );
    }

    #[test]
    fn a_well_formed_master_produces_no_structural_findings() {
        let master = parse_master(
            "#EXTM3U\n#EXT-X-VERSION:8\n#EXT-X-INDEPENDENT-SEGMENTS\n\
             #EXT-X-DEFINE:NAME=\"host\",VALUE=\"https://cdn\"\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1000\n{$host}/v.m3u8\n",
        );
        assert!(check_master_structure(&master).is_empty());
    }

    // ── check_rendition_group_references ──────────────────────────────────────

    #[test]
    fn a_dangling_audio_group_reference_errors() {
        let master = parse_master(
            "#EXTM3U\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"en\",URI=\"a.m3u8\"\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1000,AUDIO=\"missing\"\nv.m3u8\n",
        );
        let issues = check_rendition_group_references(&master);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].severity, Severity::Error);
        assert!(issues[0].message.contains("AUDIO=\"missing\""), "{}", issues[0].message);
    }

    #[test]
    fn dangling_subtitle_and_video_group_references_error() {
        let master = parse_master(
            "#EXTM3U\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1000,SUBTITLES=\"subs\",VIDEO=\"alt\"\nv.m3u8\n",
        );
        let issues = check_rendition_group_references(&master);
        assert_eq!(issues.len(), 2, "{issues:?}");
        assert!(issues.iter().any(|i| i.message.contains("SUBTITLES=\"subs\"")));
        assert!(issues.iter().any(|i| i.message.contains("VIDEO=\"alt\"")));
    }

    #[test]
    fn resolvable_group_references_and_closed_captions_none_pass() {
        let master = parse_master(
            "#EXTM3U\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"en\",URI=\"a.m3u8\"\n\
             #EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"en\",URI=\"s.m3u8\"\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1000,AUDIO=\"aac\",SUBTITLES=\"subs\",\
             CLOSED-CAPTIONS=NONE\nv.m3u8\n",
        );
        let issues = check_rendition_group_references(&master);
        assert!(issues.is_empty(), "CLOSED-CAPTIONS=NONE is not a group reference: {issues:?}");
    }

    // ── check_interstitials ───────────────────────────────────────────────────

    #[test]
    fn interstitial_valid_with_asset_uri() {
        let content = concat!(
            "#EXTM3U\n",
            "#EXT-X-DATERANGE:",
            r#"ID="ad-1",START-DATE="2024-01-15T12:00:00Z","#,
            r#"CLASS="com.apple.hls.interstitial","#,
            "X-ASSET-URI=\"https://ads.example.com/ad.m3u8\"\n",
        );
        let pl = make_playlist("v", content);
        let (issues, interstitials) = check_interstitials(&[pl]);
        assert!(issues.is_empty(), "valid interstitial must produce no issues: {:?}", issues);
        assert_eq!(interstitials.len(), 1);
        assert_eq!(interstitials[0].id, "ad-1");
        assert!(interstitials[0].asset_uri.is_some());
    }

    #[test]
    fn interstitial_missing_id_errors() {
        let content = concat!(
            "#EXTM3U\n",
            "#EXT-X-DATERANGE:",
            r#"START-DATE="2024-01-15T12:00:00Z","#,
            r#"CLASS="com.apple.hls.interstitial","#,
            "X-ASSET-URI=\"https://ads.example.com/ad.m3u8\"\n",
        );
        let pl = make_playlist("v", content);
        let (issues, _) = check_interstitials(&[pl]);
        assert!(issues.iter().any(|i| i.severity == Severity::Error && i.message.contains("missing ID")));
    }

    #[test]
    fn interstitial_missing_asset_errors() {
        let content = concat!(
            "#EXTM3U\n",
            "#EXT-X-DATERANGE:",
            r#"ID="ad-1",START-DATE="2024-01-15T12:00:00Z","#,
            "CLASS=\"com.apple.hls.interstitial\"\n",
        );
        let pl = make_playlist("v", content);
        let (issues, _) = check_interstitials(&[pl]);
        assert!(issues.iter().any(|i| i.severity == Severity::Error && i.message.contains("X-ASSET-URI/X-ASSET-LIST")));
    }

    #[test]
    fn interstitial_both_asset_uri_and_list_errors() {
        let content = concat!(
            "#EXTM3U\n",
            "#EXT-X-DATERANGE:",
            r#"ID="ad-1",START-DATE="2024-01-15T12:00:00Z","#,
            r#"CLASS="com.apple.hls.interstitial","#,
            "X-ASSET-URI=\"https://ads.example.com/ad.m3u8\",",
            "X-ASSET-LIST=\"https://ads.example.com/ads.json\"\n",
        );
        let pl = make_playlist("v", content);
        let (issues, _) = check_interstitials(&[pl]);
        assert!(issues.iter().any(|i| i.severity == Severity::Error && i.message.contains("both X-ASSET-URI and X-ASSET-LIST")));
    }

    #[test]
    fn interstitial_invalid_snap_value_warns() {
        let content = concat!(
            "#EXTM3U\n",
            "#EXT-X-DATERANGE:",
            r#"ID="ad-1",START-DATE="2024-01-15T12:00:00Z","#,
            r#"CLASS="com.apple.hls.interstitial","#,
            "X-ASSET-URI=\"https://ads.example.com/ad.m3u8\",",
            "X-SNAP=\"BEFORE\"\n",
        );
        let pl = make_playlist("v", content);
        let (issues, _) = check_interstitials(&[pl]);
        assert!(issues.iter().any(|i| i.severity == Severity::Warn && i.message.contains("X-SNAP")));
    }
    /// Regression: a second EXT-X-DATERANGE with the same ID and CLASS
    /// (per rfc8216bis §D.2 this is an "update" tag) must NOT trigger a
    /// false "Missing X-ASSET-URI or X-ASSET-LIST" error.
    #[test]
    fn interstitial_update_tag_no_false_asset_error() {
        // First DATERANGE: the real "OUT" tag with X-ASSET-LIST.
        // Second DATERANGE: same ID + CLASS, no asset attrs (update / IN tag).
        let content = concat!(
            "#EXTM3U\n",
            "#EXT-X-DATERANGE:",
            "ID=\"ad-sle-1\",",
            "START-DATE=\"2024-01-15T12:00:00Z\",",
            "CLASS=\"com.apple.hls.interstitial\",",
            "PLANNED-DURATION=30.0,",
            "X-ASSET-LIST=\"https://ads.example.com/assets.json\"\n",
            // Update tag — same ID, same CLASS, no asset attributes
            "#EXT-X-DATERANGE:",
            "ID=\"ad-sle-1\",",
            "START-DATE=\"2024-01-15T12:00:00Z\",",
            "CLASS=\"com.apple.hls.interstitial\",",
            "X-RESUME-OFFSET=0\n",
        );
        let pl = make_playlist("v1", content);
        let (issues, interstitials) = check_interstitials(&[pl]);
        // No errors — the update tag must not trigger a false positive
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(
            errors.is_empty(),
            "update DATERANGE tag must not produce errors, got: {:?}", errors
        );
        // Only one Interstitial should be created (the first occurrence)
        assert_eq!(interstitials.len(), 1, "update tag must not create a second Interstitial");
        assert!(interstitials[0].asset_list.is_some());
    }

}