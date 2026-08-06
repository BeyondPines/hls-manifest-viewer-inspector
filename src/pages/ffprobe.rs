use crate::utils::{
    href::playlist_href,
    mp4_atom_properties::{AtomPropertyValue, get_properties},
    network::{FetchError, fetch_array_buffer, fetch_text},
};
use url::Url;
use leptos::prelude::*;
use mp4_atom::{Header, ReadFrom};
use quick_m3u8::{
    HlsLine, Reader,
    tag::{KnownTag, HlsPlaylistType, hls::Tag},
};
use std::{collections::HashSet, io::Cursor};

// ── Constants ────────────────────────────────────────────────────────────────


// ── Data types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ProbeReport {
    pub url: String,
    pub manifest_type: String,
    pub format_name: Option<String>,
    pub major_brand: Option<String>,
    pub duration_s: Option<f64>,
    pub stream_count: usize,
    pub session_tags: Vec<(String, String)>,
    pub hls_version: Option<u32>,
    pub target_duration: Option<f64>,
    pub playlist_type: Option<String>,
    pub is_live: bool,
    pub total_segments: usize,
    pub ll_hls: Option<LlHlsInfo>,
    pub video_tracks: Vec<VideoTrackInfo>,
    pub audio_tracks: Vec<AudioTrackInfo>,
    pub subtitle_tracks: Vec<SubtitleTrackInfo>,
    pub caption_tracks: Vec<CaptionTrackInfo>,
    pub encryption_methods: Vec<String>,
    pub key_formats: Vec<String>,
    pub drm_systems: Vec<DrmInfo>,
    pub init_segment_probed: bool,
    pub probe_notes: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LlHlsInfo {
    pub part_hold_back: Option<f64>,
    pub can_skip_until: Option<f64>,
    pub can_block_reload: bool,
}

#[derive(Debug, Clone, Default)]
pub struct VideoTrackInfo {
    pub name: String,
    pub codec: Option<String>,
    pub codec_long: Option<String>,
    pub profile: Option<String>,
    pub level: Option<String>,
    pub resolution: Option<String>,
    pub frame_rate: Option<f64>,
    pub bitrate_bps: Option<u64>,
    pub color_space: Option<String>,
    pub hdr_format: Option<String>,
    pub color_primaries: Option<String>,
    pub transfer_characteristics: Option<String>,
    pub matrix_coefficients: Option<String>,
    pub full_range: Option<bool>,
    pub bit_depth: Option<u8>,
    pub pixel_format: Option<String>,
    pub sar: Option<String>,
    pub dar: Option<String>,
    pub init_uri: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AudioTrackInfo {
    pub name: String,
    pub group_id: String,
    pub codec: Option<String>,
    pub codec_long: Option<String>,
    pub channels: Option<u32>,
    pub channel_layout: Option<String>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u16>,
    pub bitrate_bps: Option<u64>,
    pub language: Option<String>,
    pub is_default: bool,
    /// Absolute URI of this rendition's media playlist (None for muxed audio)
    pub playlist_uri: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SubtitleTrackInfo {
    pub name: String,
    pub language: Option<String>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CaptionTrackInfo {
    pub name: String,
    pub group_id: String,
    pub language: Option<String>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DrmInfo {
    pub system_name: String,
    pub system_id: String,
}

#[derive(Default)]
struct Mp4ProbeInfo {
    major_brand: Option<String>,
    video_codec_override: Option<String>,
    video_profile: Option<String>,
    video_level: Option<String>,
    video_bit_depth: Option<u8>,
    video_color_primaries: Option<String>,
    video_transfer_char: Option<String>,
    video_matrix_coefficients: Option<String>,
    video_full_range: Option<bool>,
    video_pixel_format: Option<String>,
    audio_sample_rate: Option<u32>,
    audio_channels: Option<u16>,
    audio_bit_depth: Option<u16>,
    drm_systems: Vec<DrmInfo>,
}


// ── Check category + item definitions ────────────────────────────────────────

pub struct CheckCat {
    pub id: &'static str,
    pub icon: &'static str,
    pub label: &'static str,
    pub items: &'static [CheckItem],
}

pub struct CheckItem {
    pub id: &'static str,
    pub label: &'static str,
    pub note: Option<&'static str>, // shown in parentheses
}

pub static CATEGORIES: &[CheckCat] = &[
    CheckCat {
        id: "format", icon: "📦", label: "Format & Container",
        items: &[
            CheckItem { id: "format_name",  label: "Format / Container type", note: None },
            CheckItem { id: "duration",     label: "Duration",                note: None },
            CheckItem { id: "bitrate",      label: "Overall bitrate",         note: None },
            CheckItem { id: "stream_count", label: "Number of streams",       note: None },
            CheckItem { id: "session_tags", label: "Tags / metadata",         note: None },
        ],
    },
    CheckCat {
        id: "video", icon: "🎬", label: "Video Stream",
        items: &[
            CheckItem { id: "video_codec",        label: "Codec",                    note: None },
            CheckItem { id: "video_resolution",   label: "Resolution",               note: None },
            CheckItem { id: "video_frame_rate",   label: "Frame rate",               note: None },
            CheckItem { id: "video_bitrate",      label: "Bitrate",                  note: None },
            CheckItem { id: "video_color_space",  label: "Color space / HDR format", note: None },
            CheckItem { id: "video_aspect_ratio", label: "Aspect ratio (SAR / DAR)", note: None },
            CheckItem { id: "video_profile",  label: "Profile",                  note: Some("init seg") },
            CheckItem { id: "video_level",    label: "Level",                    note: Some("init seg") },
            CheckItem { id: "video_bit_depth",label: "Bit depth",                note: Some("init seg") },
            CheckItem { id: "video_primaries",label: "Color primaries",          note: Some("init seg") },
            CheckItem { id: "video_transfer", label: "Transfer characteristics", note: Some("init seg") },
            CheckItem { id: "video_matrix",   label: "Matrix coefficients",      note: Some("init seg") },
            CheckItem { id: "video_pixel_fmt",label: "Pixel format",             note: Some("init seg") },
        ],
    },
    CheckCat {
        id: "audio", icon: "🔊", label: "Audio Stream",
        items: &[
            CheckItem { id: "audio_codec",    label: "Codec",          note: None },
            CheckItem { id: "audio_channels", label: "Channels",       note: None },
            CheckItem { id: "audio_layout",   label: "Channel layout", note: None },
            CheckItem { id: "audio_language", label: "Language",       note: None },
            CheckItem { id: "audio_rate",  label: "Sample rate", note: Some("init seg") },
            CheckItem { id: "audio_depth", label: "Bit depth",   note: Some("init seg") },
        ],
    },
    CheckCat {
        id: "subtitles", icon: "📝", label: "Subtitles & Captions",
        items: &[
            CheckItem { id: "subtitle_tracks", label: "Subtitle tracks",       note: None },
            CheckItem { id: "subtitle_lang",   label: "Subtitle languages",    note: None },
            CheckItem { id: "caption_tracks",  label: "Closed caption tracks", note: None },
        ],
    },
    CheckCat {
        id: "encryption", icon: "🔒", label: "Encryption & DRM",
        items: &[
            CheckItem { id: "enc_method",  label: "Encryption method",  note: None },
            CheckItem { id: "key_format",  label: "Key format",         note: None },
            CheckItem { id: "drm_systems", label: "DRM systems", note: Some("PSSH / EXT-X-KEY") },
        ],
    },
    CheckCat {
        id: "protocol", icon: "📡", label: "HLS Protocol",
        items: &[
            CheckItem { id: "hls_version",     label: "HLS version",             note: None },
            CheckItem { id: "target_duration", label: "Target segment duration", note: None },
            CheckItem { id: "playlist_type",   label: "Playlist type (VOD/Live)",note: None },
            CheckItem { id: "ll_hls",          label: "Low-Latency HLS",         note: None },
            CheckItem { id: "segment_count",   label: "Segment count",           note: None },
        ],
    },
];

fn all_check_ids() -> HashSet<String> {
    CATEGORIES.iter()
        .flat_map(|cat| cat.items.iter().map(|it| it.id.to_string()))
        .collect()
}

// ── Helper functions ─────────────────────────────────────────────────────────

fn prop_str(val: &AtomPropertyValue) -> String {
    match val {
        AtomPropertyValue::Basic(b) => String::from(b),
        AtomPropertyValue::Table(_) => String::new(),
    }
}

fn gcd(a: u32, b: u32) -> u32 { if b == 0 { a } else { gcd(b, a % b) }  }

fn parse_video_codec(codecs: &str) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
    // returns (short, long, profile, level)
    for codec in codecs.split(',') {
        let t = codec.trim().to_lowercase();
        if t.starts_with("avc1") || t.starts_with("avc3") {
            let parts: Vec<&str> = t.splitn(2, '.').collect();
            if parts.len() == 2 && parts[1].len() >= 6 {
                let h = parts[1];
                if let (Ok(p), Ok(l)) = (u8::from_str_radix(&h[0..2], 16), u8::from_str_radix(&h[4..6], 16)) {
                    let prof = match p {
                        0x42 => "Baseline", 0x4D => "Main", 0x58 => "Extended",
                        0x64 => "High", 0x6E => "High 10", 0x7A => "High 4:2:2",
                        0x8A | 0xF4 => "High 4:4:4", _ => "Unknown",
                    };
                    let lev = format!("{:.1}", l as f32 / 10.0);
                    return (Some("H.264".into()), Some(format!("H.264 / AVC ({} Profile, Level {})", prof, lev)),
                            Some(prof.into()), Some(lev));
                }
            }
            return (Some("H.264".into()), Some("H.264 / AVC".into()), None, None);
        }
        if t.starts_with("hvc1") || t.starts_with("hev1") {
            return (Some("H.265".into()), Some("H.265 / HEVC".into()), None, None);
        }
        if t.starts_with("dvhe") || t.starts_with("dvh1") {
            return (Some("H.265".into()), Some("Dolby Vision (HEVC)".into()), None, None);
        }
        if t.starts_with("dav1") || t.starts_with("dva1") || t.starts_with("dvav") {
            return (Some("AV1".into()), Some("Dolby Vision (AV1)".into()), None, None);
        }
        if t.starts_with("av01") { return (Some("AV1".into()), Some("AV1".into()), None, None); }
        if t.starts_with("vp09") || t.starts_with("vp9") {
            return (Some("VP9".into()), Some("VP9".into()), None, None);
        }
        if t.starts_with("vp08") { return (Some("VP8".into()), Some("VP8".into()), None, None); }
    }
    (None, None, None, None)
}

fn parse_audio_codec(codecs: &str) -> (Option<String>, Option<String>) {
    for codec in codecs.split(',') {
        let t = codec.trim().to_lowercase();
        if t.starts_with("mp4a.40.29") { return (Some("HE-AAC v2".into()), Some("HE-AAC v2 (eAAC+)".into())); }
        if t.starts_with("mp4a.40.5")  { return (Some("HE-AAC".into()),    Some("HE-AAC (AAC+)".into())); }
        if t.starts_with("mp4a")       { return (Some("AAC".into()),        Some("AAC-LC".into())); }
        if t.starts_with("ec-3") || t.starts_with("ec3") {
            return (Some("E-AC-3".into()), Some("E-AC-3 (Dolby Digital Plus)".into()));
        }
        if t.starts_with("ac-4") || t.starts_with("ac4") {
            return (Some("AC-4".into()), Some("Dolby AC-4".into()));
        }
        if t.starts_with("ac-3") || t.starts_with("ac3") {
            return (Some("AC-3".into()), Some("AC-3 (Dolby Digital)".into()));
        }
        if t.starts_with("opus") { return (Some("Opus".into()), Some("Opus".into())); }
        if t.starts_with("flac") { return (Some("FLAC".into()), Some("FLAC".into())); }
        if t.starts_with("apac") { return (Some("APAC".into()), Some("Apple Positional Audio Codec".into())); }
    }
    (None, None)
}

fn channels_to_layout(n: u32) -> String {
    match n {
        1 => "mono", 2 => "stereo", 6 => "5.1", 8 => "7.1",
        n => return format!("{} ch", n),
    }.into()
}

fn color_primaries_name(n: u16) -> String {
    match n {
        1  => "BT.709",
        5  => "BT.470BG",
        6  => "BT.601 / SMPTE 170M",
        9  => "BT.2020",
        11 => "DCI P3",
        12 => "Display P3",
        _  => return format!("Unknown ({})", n),
    }.into()
}

fn transfer_name(n: u16) -> String {
    match n {
        1  => "BT.709",
        6  => "BT.601 / SMPTE 170M",
        8  => "Linear",
        13 => "sRGB",
        14 => "BT.2020 (10-bit)",
        15 => "BT.2020 (12-bit)",
        16 => "PQ (ST 2084 / HDR10)",
        18 => "HLG (ARIB STD-B67)",
        _  => return format!("Unknown ({})", n),
    }.into()
}

fn matrix_name(n: u16) -> String {
    match n {
        0  => "Identity",
        1  => "BT.709",
        6  => "BT.601 / SMPTE 170M",
        9  => "BT.2020 NCL",
        10 => "BT.2020 CL",
        _  => return format!("Unknown ({})", n),
    }.into()
}

fn chroma_pixel_fmt(chroma_idc: u8, bit_depth: u8) -> String {
    let sub = match chroma_idc { 0 => "400", 2 => "422", 3 => "444", _ => "420" };
    if bit_depth <= 8 { format!("yuv{}p", sub) } else { format!("yuv{}p{}le", sub, bit_depth) }
}

fn fmt_bps(bps: u64) -> String {
    if bps >= 1_000_000 { format!("{:.2} Mbps", bps as f64 / 1_000_000.0) }
    else { format!("{} kbps", bps / 1000) }
}

fn fmt_dur(secs: f64) -> String {
    let s = secs as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s/3600, (s%3600)/60, s%60) }
    else { format!("{}:{:02}", s/60, s%60) }
}

// ── Local HLS parsing types (replaces crate::utils::validator) ───────────────

/// A single variant stream from a master playlist.
struct VariantStream {
    uri: String,
    bandwidth: Option<u64>,
    codecs: Option<String>,
    resolution: Option<String>,
    frame_rate: Option<f64>,
    video_range: Option<String>,
    audio_group: Option<String>,
}

/// A media rendition from an EXT-X-MEDIA tag.
struct MediaRendition {
    media_type: String,
    name: String,
    group_id: String,
    uri: Option<String>,
    language: Option<String>,
    is_default: bool,
    /// Channel count extracted from the CHANNELS attribute.
    channels: Option<u32>,
}

/// Parsed representation of a master playlist.
struct MasterPlaylist {
    version: u32,
    variants: Vec<VariantStream>,
    media_renditions: Vec<MediaRendition>,
}

/// A single segment entry from a media playlist.
struct SegmentInfo {
    duration: f64,
    map_uri: Option<String>,
}

/// Parsed server control tag data.
struct ServerControlInfo {
    part_hold_back: Option<f64>,
    can_skip_until: Option<f64>,
    can_block_reload: bool,
}

/// Parsed representation of a media playlist.
struct MediaPlaylist {
    version: u32,
    target_duration: f64,
    playlist_type: Option<String>,
    has_endlist: bool,
    segments: Vec<SegmentInfo>,
    server_control: Option<ServerControlInfo>,
}

// ── HLS parsing with quick-m3u8 ──────────────────────────────────────────────

fn make_reader_opts() -> quick_m3u8::config::ParsingOptions {
    quick_m3u8::config::ParsingOptions::default()
}

fn playlist_type_str(pt: HlsPlaylistType) -> String {
    match pt {
        HlsPlaylistType::Event => "EVENT".into(),
        HlsPlaylistType::Vod => "VOD".into(),
    }
}

/// Returns `true` if the manifest text is a master (multivariant) playlist.
fn is_master_playlist(content: &str) -> bool {
    content.lines().any(|l| l.starts_with("#EXT-X-STREAM-INF"))
}

/// Resolve a potentially-relative URI against a base URL string.
fn resolve_uri(base: &str, uri: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri.to_string();
    }
    // Strip everything after the last '/' from the base to get the directory.
    let base_dir = base.rfind('/').map(|i| &base[..=i]).unwrap_or(base);
    if uri.starts_with('/') {
        // Protocol-relative or absolute path — reuse the origin of the base.
        let origin_end = base_dir.find("://")
            .and_then(|p| base_dir[p + 3..].find('/').map(|q| p + 3 + q))
            .unwrap_or(base_dir.len());
        format!("{}{}", &base_dir[..origin_end], uri)
    } else {
        format!("{}{}", base_dir, uri)
    }
}

fn parse_master_playlist(base_url: &str, content: &str) -> MasterPlaylist {
    let mut master = MasterPlaylist {
        version: 3,
        variants: Vec::new(),
        media_renditions: Vec::new(),
    };
    let opts = make_reader_opts();
    let mut pending_stream_inf: Option<VariantStream> = None;

    let mut reader = Reader::from_str(content, opts);
    loop {
        match reader.read_line() {
            Ok(Some(hls_line)) => match hls_line {
                HlsLine::KnownTag(KnownTag::Hls(Tag::Version(v))) => {
                    master.version = v.version() as u32;
                }
                HlsLine::KnownTag(KnownTag::Hls(Tag::StreamInf(si))) => {
                    let res = si.resolution().map(|r| format!("{}x{}", r.width, r.height));
                    let codecs = si.codecs().map(|c| c.to_string());
                    let video_range = si.video_range().map(|vr| vr.to_string());
                    let audio_group = si.audio().map(|a| a.to_string());
                    pending_stream_inf = Some(VariantStream {
                        uri: String::new(),
                        bandwidth: Some(si.bandwidth()),
                        codecs,
                        resolution: res,
                        frame_rate: si.frame_rate(),
                        video_range,
                        audio_group,
                    });
                }
                HlsLine::Uri(uri) => {
                    if let Some(mut vs) = pending_stream_inf.take() {
                        vs.uri = resolve_uri(base_url, &uri);
                        master.variants.push(vs);
                    }
                }
                HlsLine::KnownTag(KnownTag::Hls(Tag::Media(m))) => {
                    // Extract numeric channel count from the CHANNELS attribute.
                    // `valid()` borrows the `Channels` value, so we must call
                    // `count()` (returning a Copy u32) inside the same closure.
                    let channels = m.channels()
                        .and_then(|c| c.valid().map(|v| v.count()));
                    master.media_renditions.push(MediaRendition {
                        media_type: m.media_type().to_string(),
                        name: m.name().to_string(),
                        group_id: m.group_id().to_string(),
                        uri: m.uri().map(|u| resolve_uri(base_url, u)),
                        language: m.language().map(|l| l.to_string()),
                        is_default: m.default(),
                        channels,
                    });
                }
                _ => {}
            },
            Ok(None) => break,
            Err(_) => break,
        }
    }
    master
}

fn parse_media_playlist(base_url: &str, content: &str) -> MediaPlaylist {
    let mut pl = MediaPlaylist {
        version: 3,
        target_duration: 0.0,
        playlist_type: None,
        has_endlist: false,
        segments: Vec::new(),
        server_control: None,
    };
    let opts = make_reader_opts();
    let mut pending_duration: Option<f64> = None;
    let mut current_map_uri: Option<String> = None;

    let mut reader = Reader::from_str(content, opts);
    loop {
        match reader.read_line() {
            Ok(Some(hls_line)) => match hls_line {
                HlsLine::KnownTag(KnownTag::Hls(Tag::Version(v))) => {
                    pl.version = v.version() as u32;
                }
                HlsLine::KnownTag(KnownTag::Hls(Tag::Targetduration(td))) => {
                    pl.target_duration = td.target_duration() as f64;
                }
                HlsLine::KnownTag(KnownTag::Hls(Tag::PlaylistType(pt))) => {
                    pl.playlist_type = Some(playlist_type_str(pt.playlist_type()));
                }
                HlsLine::KnownTag(KnownTag::Hls(Tag::Endlist(_))) => {
                    pl.has_endlist = true;
                }
                HlsLine::KnownTag(KnownTag::Hls(Tag::Map(m))) => {
                    current_map_uri = Some(resolve_uri(base_url, m.uri()));
                }
                HlsLine::KnownTag(KnownTag::Hls(Tag::Inf(inf))) => {
                    pending_duration = Some(inf.duration());
                }
                HlsLine::Uri(_uri) => {
                    if let Some(dur) = pending_duration.take() {
                        pl.segments.push(SegmentInfo {
                            duration: dur,
                            map_uri: current_map_uri.clone(),
                        });
                    }
                }
                HlsLine::KnownTag(KnownTag::Hls(Tag::ServerControl(sc))) => {
                    pl.server_control = Some(ServerControlInfo {
                        part_hold_back: sc.part_hold_back(),
                        can_skip_until: sc.can_skip_until(),
                        can_block_reload: sc.can_block_reload(),
                    });
                }
                _ => {}
            },
            Ok(None) => break,
            Err(_) => break,
        }
    }
    pl
}

fn parse_session_data(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let opts = make_reader_opts();
    let mut reader = Reader::from_str(content, opts);
    loop {
        match reader.read_line() {
            Ok(Some(HlsLine::KnownTag(KnownTag::Hls(Tag::SessionData(sd))))) => {
                let k = sd.data_id().to_string();
                let v = sd.value().map(|s| s.to_string())
                    .or_else(|| sd.uri().map(|s| s.to_string()))
                    .unwrap_or_default();
                if !k.is_empty() { out.push((k, v)); }
            }
            Ok(None) | Err(_) => break,
            Ok(Some(_)) => {}
        }
    }
    out
}

fn parse_key_info(content: &str) -> (Vec<String>, Vec<String>) {
    let mut methods: Vec<String> = Vec::new();
    let mut formats: Vec<String> = Vec::new();
    let opts = make_reader_opts();
    let mut reader = Reader::from_str(content, opts);
    loop {
        match reader.read_line() {
            Ok(Some(HlsLine::KnownTag(KnownTag::Hls(Tag::Key(k))))) => {
                let m = k.method().to_string();
                if !methods.contains(&m) { methods.push(m); }
                let kf = k.keyformat().to_string();
                if !formats.contains(&kf) { formats.push(kf); }
            }
            Ok(None) | Err(_) => break,
            Ok(Some(_)) => {}
        }
    }
    (methods, formats)
}

/// Derive DRM system names from EXT-X-KEY KEYFORMAT values.
/// FairPlay does not embed PSSH boxes in the init segment — its DRM identity
/// lives entirely in the KEYFORMAT attribute of EXT-X-KEY.
fn drm_from_key_formats(formats: &[String]) -> Vec<DrmInfo> {
    formats.iter().filter_map(|kf| match kf.as_str() {
        "com.apple.streamingkeydelivery" => Some(DrmInfo {
            system_name: "Apple FairPlay".into(),
            system_id: "94ce86fb-07ff-4f43-adb8-93d2fa968ca2".into(),
        }),
        "urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed" => Some(DrmInfo {
            system_name: "Google Widevine".into(),
            system_id: "edef8ba9-79d6-4ace-a3c8-27dcd51d21ed".into(),
        }),
        "com.microsoft.playready" => Some(DrmInfo {
            system_name: "Microsoft PlayReady".into(),
            system_id: "9a04f079-9840-4286-ab92-e65be0885f95".into(),
        }),
        "urn:uuid:f239e769-efa3-4850-9c16-a903c6932efb" => Some(DrmInfo {
            system_name: "Adobe Primetime DRM".into(),
            system_id: "f239e769-efa3-4850-9c16-a903c6932efb".into(),
        }),
        _ => None,
    }).collect()
}

// ── MP4 init-segment prober ──────────────────────────────────────────────────

fn probe_mp4(data: Vec<u8>) -> Mp4ProbeInfo {
    let mut info = Mp4ProbeInfo::default();
    let mut reader = Cursor::new(data);
    let mut container_ends: Vec<u64> = Vec::new();
    let mut in_video = false;
    let mut in_audio = false;

    loop {
        while let Some(&end) = container_ends.last() {
            if reader.position() >= end { container_ends.pop(); } else { break; }
        }
        if reader.position() as usize >= reader.get_ref().len() { break; }
        let Ok(header) = Header::read_from(&mut reader) else { break; };
        let Ok(atom)   = get_properties(&header, &mut reader) else { break; };
        if let Some(e) = atom.new_depth_until { container_ends.push(e); }

        let props = &atom.properties;
        let get = |key: &str| -> Option<String> {
            props.properties.iter()
                .find(|(k, _)| k.as_ref() == key)
                .map(|(_, v)| prop_str(v))
                .filter(|s| !s.is_empty())
        };

        match props.box_name {
            "FileTypeBox" => {
                info.major_brand = get("major_brand");
            }
            "HandlerBox" => {
                match get("handler").as_deref() {
                    Some("vide") => { in_video = true;  in_audio = false; }
                    Some("soun") => { in_audio = true;  in_video = false; }
                    _            => { in_video = false; in_audio = false; }
                }
            }
            "AVCSampleEntryBox" | "EncryptedVisualSampleEntryBox" if in_video || info.video_codec_override.is_none() => {
                if props.box_name == "AVCSampleEntryBox" {
                    info.video_codec_override = Some("H.264".into());
                }
            }
            "HEVCSampleEntryBox" if in_video || info.video_codec_override.is_none() => {
                info.video_codec_override = Some("H.265 / HEVC".into());
            }
            "DolbyVisionHEVCSampleEntryBox" | "DolbyVisionHVC1SampleEntryBox" => {
                info.video_codec_override = Some("Dolby Vision (HEVC)".into());
            }
            "AV1SampleEntryBox"  if in_video || info.video_codec_override.is_none() => {
                info.video_codec_override = Some("AV1".into());
            }
            "VP09SampleEntryBox" if in_video || info.video_codec_override.is_none() => {
                info.video_codec_override = Some("VP9".into());
            }
            "VP08SampleEntryBox" if in_video || info.video_codec_override.is_none() => {
                info.video_codec_override = Some("VP8".into());
            }
            "MP4AudioSampleEntryBox" | "EncryptedAudioSampleEntryBox" if in_audio => {
                if let Some(ch) = get("channel_count") { info.audio_channels = ch.parse().ok(); }
                if let Some(bd) = get("sample_size")   { info.audio_bit_depth = bd.parse().ok(); }
                if let Some(sr) = get("sample_rate") {
                    let sr = sr.split('.').next().unwrap_or("").to_string();
                    info.audio_sample_rate = sr.parse().ok();
                }
            }
            "OpusSampleEntryBox" if in_audio => {
                if let Some(ch) = get("channel_count") { info.audio_channels = ch.parse().ok(); }
            }
            "EC3SampleEntryBox" if in_audio => {
                if let Some(ch) = get("channel_count") { info.audio_channels = ch.parse().ok(); }
            }
            "AC3SampleEntryBox" | "AC4SampleEntryBox" if in_audio => {
                if let Some(ch) = get("channel_count") { info.audio_channels = ch.parse().ok(); }
            }
            "ApacSampleEntryBox" if in_audio => {
                if let Some(ch) = get("channel_count") { info.audio_channels = ch.parse().ok(); }
                if let Some(bd) = get("sample_size")   { info.audio_bit_depth = bd.parse().ok(); }
                if let Some(sr) = get("sample_rate") {
                    let sr = sr.split('.').next().unwrap_or("").to_string();
                    info.audio_sample_rate = sr.parse().ok();
                }
            }
            "AVCConfigurationBox" if in_video => {
                if let Some(p) = get("avc_profile_indication").and_then(|s| s.parse::<u8>().ok()) {
                    info.video_profile = Some(match p {
                        0x42 => "Baseline", 0x4D => "Main", 0x58 => "Extended",
                        0x64 => "High", 0x6E => "High 10", 0x7A => "High 4:2:2",
                        0x8A | 0xF4 => "High 4:4:4", _ => "Unknown",
                    }.into());
                }
                if let Some(l) = get("avc_level_indication").and_then(|s| s.parse::<u8>().ok()) {
                    info.video_level = Some(format!("{:.1}", l as f32 / 10.0));
                }
                if let Some(bd) = get("ext_bit_depth_luma").and_then(|s| s.parse::<u8>().ok()) {
                    if bd > 0 { info.video_bit_depth = Some(bd); }
                }
                if info.video_bit_depth.is_none() { info.video_bit_depth = Some(8); }
                if info.video_pixel_format.is_none() { info.video_pixel_format = Some("yuv420p".into()); }
            }
            "HEVCConfigurationBox" if in_video => {
                if let Some(p) = get("general_profile_idc").and_then(|s| s.parse::<u8>().ok()) {
                    info.video_profile = Some(match p {
                        1 => "Main", 2 => "Main 10", 3 => "Main Still Picture",
                        4 => "Format Range Extensions", _ => "Unknown",
                    }.into());
                }
                if let Some(l) = get("general_level_idc").and_then(|s| s.parse::<u8>().ok()) {
                    info.video_level = Some(format!("{:.1}", l as f32 / 30.0));
                }
                if let Some(bd) = get("bit_depth_luma_minus8").and_then(|s| s.parse::<u8>().ok()) {
                    info.video_bit_depth = Some(8 + bd);
                }
                if let Some(cf) = get("chroma_format_idc").and_then(|s| s.parse::<u8>().ok()) {
                    let bd = info.video_bit_depth.unwrap_or(8);
                    info.video_pixel_format = Some(chroma_pixel_fmt(cf, bd));
                }
            }
            "ColourInformationBox" => {
                if let Some(n) = get("colour_primaries").and_then(|s| s.parse::<u16>().ok()) {
                    info.video_color_primaries = Some(color_primaries_name(n));
                }
                if let Some(n) = get("transfer_characteristics").and_then(|s| s.parse::<u16>().ok()) {
                    info.video_transfer_char = Some(transfer_name(n));
                }
                if let Some(n) = get("matrix_coefficients").and_then(|s| s.parse::<u16>().ok()) {
                    info.video_matrix_coefficients = Some(matrix_name(n));
                }
                if let Some(fr) = get("full_range_flag") { info.video_full_range = Some(fr == "true"); }
                // QuickTime nclc
                if let Some(n) = get("primaries_index").and_then(|s| s.parse::<u16>().ok()) {
                    info.video_color_primaries = Some(color_primaries_name(n));
                }
                if let Some(n) = get("transfer_function_index").and_then(|s| s.parse::<u16>().ok()) {
                    info.video_transfer_char = Some(transfer_name(n));
                }
                if let Some(n) = get("matrix_index").and_then(|s| s.parse::<u16>().ok()) {
                    info.video_matrix_coefficients = Some(matrix_name(n));
                }
            }
            "ProtectionSystemSpecificHeaderBox" => {
                let sys = get("system_ref").unwrap_or_default();
                let id  = get("system_id").unwrap_or_default();
                if !sys.is_empty() || !id.is_empty() {
                    info.drm_systems.push(DrmInfo { system_name: sys, system_id: id });
                }
            }
            _ => {}
        }
    }
    info
}

// ── Main probe function ───────────────────────────────────────────────────────

async fn probe_stream(url: &str, selected: &HashSet<String>) -> Result<ProbeReport, FetchError> {
    let mut r = ProbeReport { url: url.to_string(), ..Default::default() };

    let needs_init = selected.iter().any(|id| {
        matches!(id.as_str(),
            "video_profile"|"video_level"|"video_bit_depth"|"video_primaries"|
            "video_transfer"|"video_matrix"|"video_pixel_fmt"|
            "audio_rate"|"audio_depth"|"drm_systems")
    });

    let resp = fetch_text(url.to_string()).await?;
    let content = &resp.response_text;

    r.format_name = Some("HLS".into());

    if is_master_playlist(content) {
        r.manifest_type = "HLS Multivariant Playlist".into();
        let master = parse_master_playlist(url, content);
        r.hls_version = Some(master.version);
        r.session_tags = parse_session_data(content);
        r.stream_count = master.variants.len() + master.media_renditions.len();

        // Collect audio codec mappings (group_id → codec)
        let audio_codec_map: std::collections::HashMap<String, String> = master.variants.iter()
            .filter_map(|v| {
                let grp = v.audio_group.as_ref()?;
                let codecs = v.codecs.as_ref()?;
                let audio = codecs.split(',').nth(1).map(|c| c.trim().to_string())?;
                Some((grp.clone(), audio))
            })
            .collect();

        // Video tracks
        for variant in &master.variants {
            let mut vt = VideoTrackInfo::default();
            vt.bitrate_bps = variant.bandwidth;
            if let Some(res) = &variant.resolution {
                vt.resolution = Some(res.clone());
                let pts: Vec<&str> = res.split('x').collect();
                if pts.len() == 2 {
                    if let (Ok(w), Ok(h)) = (pts[0].parse::<u32>(), pts[1].parse::<u32>()) {
                        let g = gcd(w, h);
                        vt.dar = Some(format!("{}:{}", w/g, h/g));
                        vt.sar = Some("1:1".into());
                    }
                }
            }
            vt.frame_rate = variant.frame_rate;
            if let Some(codecs) = &variant.codecs {
                let (short, long, prof, lev) = parse_video_codec(codecs);
                vt.codec = short; vt.codec_long = long;
                vt.profile = prof; vt.level = lev;
            }
            vt.color_space = variant.video_range.clone();
            vt.hdr_format = match variant.video_range.as_deref() {
                Some("PQ") => Some("HDR10 / Dolby Vision".into()),
                Some("HLG") => Some("HLG".into()),
                _ => None,
            };
            // Set colour defaults inferred from VIDEO-RANGE.  The init-segment
            // probe will override these with exact colr-box values when present;
            // when the init segment omits the colr box (common for SDR BT.709
            // content) these defaults remain and are still correct.
            let (primaries, transfer, matrix) = match variant.video_range.as_deref() {
                Some("PQ")  => ("BT.2020", "PQ (ST 2084 / HDR10)", "BT.2020 NCL"),
                Some("HLG") => ("BT.2020", "HLG (ARIB STD-B67)",   "BT.2020 NCL"),
                _           => ("BT.709",  "BT.709",                "BT.709"),
            };
            vt.color_primaries          = Some(primaries.into());
            vt.transfer_characteristics = Some(transfer.into());
            vt.matrix_coefficients      = Some(matrix.into());
            vt.name = vt.resolution.clone()
                .unwrap_or_else(|| vt.bitrate_bps.map(|b| format!("{}k", b/1000)).unwrap_or_default());
            vt.init_uri = Some(variant.uri.clone());
            r.video_tracks.push(vt);
        }

        // Audio tracks
        for rend in master.media_renditions.iter().filter(|r| r.media_type == "AUDIO") {
            let mut at = AudioTrackInfo::default();
            at.name = rend.name.clone();
            at.group_id = rend.group_id.clone();
            at.language = rend.language.clone();
            at.is_default = rend.is_default;
            at.playlist_uri = rend.uri.clone();
            if let Some(n) = rend.channels {
                at.channels = Some(n);
                at.channel_layout = Some(channels_to_layout(n));
            }
            // Codec from stream-inf codecs map
            if let Some(raw) = audio_codec_map.get(&rend.group_id) {
                let (s, l) = parse_audio_codec(raw);
                at.codec = s; at.codec_long = l;
            }
            r.audio_tracks.push(at);
        }

        // Subtitle tracks
        for rend in master.media_renditions.iter().filter(|r| r.media_type == "SUBTITLES") {
            r.subtitle_tracks.push(SubtitleTrackInfo {
                name: rend.name.clone(),
                language: rend.language.clone(),
                is_default: rend.is_default,
                ..Default::default()
            });
        }

        // Caption tracks
        for rend in master.media_renditions.iter().filter(|r| r.media_type == "CLOSED-CAPTIONS") {
            r.caption_tracks.push(CaptionTrackInfo {
                name: rend.name.clone(),
                group_id: rend.group_id.clone(),
                language: rend.language.clone(),
                is_default: rend.is_default,
            });
        }

        // Probe first variant media playlist
        if let Some(first) = master.variants.first() {
            if let Ok(mr) = fetch_text(first.uri.clone()).await {
                let pl = parse_media_playlist(&first.uri, &mr.response_text);
                r.target_duration = Some(pl.target_duration);
                r.playlist_type = pl.playlist_type.clone();
                r.is_live = !pl.has_endlist;
                r.total_segments = pl.segments.len();
                r.duration_s = Some(pl.segments.iter().map(|s| s.duration).sum());
                let (methods, fmts) = parse_key_info(&mr.response_text);
                r.encryption_methods = methods;
                r.key_formats = fmts.clone();
                // Detect DRM from EXT-X-KEY KEYFORMAT (FairPlay carries no PSSH)
                for d in drm_from_key_formats(&fmts) {
                    if !r.drm_systems.iter().any(|e| e.system_id == d.system_id) {
                        r.drm_systems.push(d);
                    }
                }
                if let Some(sc) = &pl.server_control {
                    r.ll_hls = Some(LlHlsInfo {
                        part_hold_back: sc.part_hold_back,
                        can_skip_until: sc.can_skip_until,
                        can_block_reload: sc.can_block_reload,
                    });
                }
                // Init segment probe
                if needs_init {
                    let init_url = pl.segments.iter().find_map(|s| s.map_uri.clone());
                    if let Some(iurl) = init_url {
                        match fetch_array_buffer(iurl, None).await {
                            Ok(resp) => {
                                let mp4 = probe_mp4(resp.response_body);
                                r.major_brand = mp4.major_brand;

                                // Colour space / HDR properties and bit depth are stream-level
                                // (same encoding settings across all renditions). Propagate to
                                // every video track so no rendition shows "—" for these fields.
                                for vt in r.video_tracks.iter_mut() {
                                    // Codec override only if CODECS string didn't already resolve it
                                    if vt.codec.is_none() {
                                        if let Some(c) = &mp4.video_codec_override { vt.codec = Some(c.clone()); }
                                    }
                                    // Profile / level: fill from init segment only when the
                                    // CODECS attribute didn't supply them
                                    if vt.profile.is_none() { vt.profile = mp4.video_profile.clone(); }
                                    if vt.level.is_none()   { vt.level   = mp4.video_level.clone();   }
                                    // Override defaults only when the colr box was present —
                                    // use if-let so a missing colr box doesn't erase the
                                    // VIDEO-RANGE defaults set when building the track list.
                                    if let Some(v) = mp4.video_bit_depth               { vt.bit_depth               = Some(v); }
                                    if let Some(v) = mp4.video_color_primaries.clone()  { vt.color_primaries         = Some(v); }
                                    if let Some(v) = mp4.video_transfer_char.clone()    { vt.transfer_characteristics= Some(v); }
                                    if let Some(v) = mp4.video_matrix_coefficients.clone() { vt.matrix_coefficients = Some(v); }
                                    if let Some(v) = mp4.video_full_range               { vt.full_range              = Some(v); }
                                    if let Some(v) = mp4.video_pixel_format.clone()     { vt.pixel_format            = Some(v); }
                                }

                                // Frame rate: fill in from the probed rendition for any track
                                // missing it (FRAME-RATE is optional in EXT-X-STREAM-INF for ≤30 fps)
                                let probed_fps = r.video_tracks.first().and_then(|vt| vt.frame_rate);
                                if let Some(fps) = probed_fps {
                                    for vt in r.video_tracks.iter_mut().skip(1) {
                                        if vt.frame_rate.is_none() { vt.frame_rate = Some(fps); }
                                    }
                                }

                                // Muxed streams: video init segment may also carry audio
                                // (applies if audio_* fields are populated from the probe above)
                                for at in r.audio_tracks.iter_mut() {
                                    if at.sample_rate.is_none() { at.sample_rate = mp4.audio_sample_rate; }
                                    if at.bit_depth.is_none()   { at.bit_depth   = mp4.audio_bit_depth;   }
                                    if at.channels.is_none() {
                                        at.channels = mp4.audio_channels.map(|c| c as u32);
                                        if let Some(n) = at.channels {
                                            at.channel_layout = Some(channels_to_layout(n));
                                        }
                                    }
                                }
                                // Merge PSSH-based findings; don't overwrite EXT-X-KEY derived ones
                                for d in mp4.drm_systems {
                                    if !r.drm_systems.iter().any(|e| e.system_id == d.system_id) {
                                        r.drm_systems.push(d);
                                    }
                                }
                                r.init_segment_probed = true;
                            }
                            Err(e) => { r.probe_notes.push(format!("Init segment fetch failed: {e}")); }
                        }
                    } else {
                        r.probe_notes.push("No EXT-X-MAP init segment found — skipping init-segment checks.".into());
                    }

                    // Demuxed HLS: the video init segment carries no audio data.
                    // Fetch the default audio rendition's own media playlist and
                    // probe its EXT-X-MAP init segment to get sample rate, bit
                    // depth, and channel count for all audio tracks.
                    let needs_audio_probe = r.audio_tracks.iter()
                        .any(|at| at.sample_rate.is_none() || at.bit_depth.is_none());
                    if needs_audio_probe {
                        let audio_pl_url = master.media_renditions.iter()
                            .filter(|rend| rend.media_type == "AUDIO" && rend.uri.is_some())
                            .find(|rend| rend.is_default)
                            .or_else(|| master.media_renditions.iter()
                                .find(|rend| rend.media_type == "AUDIO" && rend.uri.is_some()))
                            .and_then(|rend| rend.uri.clone());
                        if let Some(apl_url) = audio_pl_url {
                            if let Ok(apl_resp) = fetch_text(apl_url.clone()).await {
                                let apl = parse_media_playlist(&apl_url, &apl_resp.response_text);
                                let audio_init_url = apl.segments.iter().find_map(|s| s.map_uri.clone());
                                if let Some(aiurl) = audio_init_url {
                                    if let Ok(aresp) = fetch_array_buffer(aiurl, None).await {
                                        let amp4 = probe_mp4(aresp.response_body);
                                        for at in r.audio_tracks.iter_mut() {
                                            if at.sample_rate.is_none() { at.sample_rate = amp4.audio_sample_rate; }
                                            if at.bit_depth.is_none()   { at.bit_depth   = amp4.audio_bit_depth;   }
                                            if at.channels.is_none() {
                                                at.channels = amp4.audio_channels.map(|c| c as u32);
                                                if let Some(n) = at.channels {
                                                    at.channel_layout = Some(channels_to_layout(n));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    } else {
        // Single media playlist
        r.manifest_type = "HLS Media Playlist".into();
        let pl = parse_media_playlist(url, content);
        r.hls_version = Some(pl.version);
        r.target_duration = Some(pl.target_duration);
        r.playlist_type = pl.playlist_type.clone();
        r.is_live = !pl.has_endlist;
        r.total_segments = pl.segments.len();
        r.duration_s = Some(pl.segments.iter().map(|s| s.duration).sum());
        let (methods, fmts) = parse_key_info(content);
        r.encryption_methods = methods;
        r.key_formats = fmts.clone();
        for d in drm_from_key_formats(&fmts) {
            if !r.drm_systems.iter().any(|e| e.system_id == d.system_id) {
                r.drm_systems.push(d);
            }
        }
        if let Some(sc) = &pl.server_control {
            r.ll_hls = Some(LlHlsInfo {
                part_hold_back: sc.part_hold_back,
                can_skip_until: sc.can_skip_until,
                can_block_reload: sc.can_block_reload,
            });
        }
        r.stream_count = 1;
        r.session_tags = parse_session_data(content);
    }

    Ok(r)
}

// ── UI ────────────────────────────────────────────────────────────────────────

#[component]
pub fn Ffprobe() -> impl IntoView {
    let (url, set_url) = signal(String::new());
    let selected = RwSignal::new(all_check_ids());
    let (report, set_report) = signal(None::<ProbeReport>);
    let (error_msg, set_error_msg) = signal(None::<String>);
    let (loading, set_loading) = signal(false);

    let toggle_check = move |id: String| {
        selected.update(|s| {
            if s.contains(&id) { s.remove(&id); } else { s.insert(id); }
        });
    };

    let toggle_category = move |cat_id: &'static str| {
        let cat_items: Vec<&'static str> = CATEGORIES.iter()
            .find(|c| c.id == cat_id)
            .map(|c| c.items.iter().map(|i| i.id).collect())
            .unwrap_or_default();
        let all_selected = cat_items.iter().all(|id| selected.get_untracked().contains(*id));
        selected.update(|s| {
            for id in &cat_items {
                if all_selected { s.remove(*id); } else { s.insert(id.to_string()); }
            }
        });
    };

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let u = url.get();
        if u.is_empty() { return; }
        let sel = selected.get();
        set_loading.set(true);
        set_error_msg.set(None);
        set_report.set(None);
        leptos::task::spawn_local(async move {
            match probe_stream(&u, &sel).await {
                Ok(r)  => set_report.set(Some(r)),
                Err(e) => set_error_msg.set(Some(format!("Probe failed: {e}"))),
            }
            set_loading.set(false);
        });
    };

    view! {
        <div class="body-content" style="max-width: min(96vw, 1440px); margin-bottom: 2em;">
            <div>
                <div class="body-content">
                    <h1 class="body-content">
                        "Stream Inspector"
                    </h1>
                    <p class="body-content body-text">
                        "Enter an HLS stream URL. Choose the checks you want, then click Probe. \
                         Checks marked \u{201c}(init seg)\u{201d} will fetch the first init segment \u{2014} \
                         these work even on encrypted streams because init segments carry \
                         unencrypted codec and DRM metadata."
                    </p>
                </div>
            </div>

            // ── Input card ──────────────────────────────────────────────────
            <div style="background: var(--color-white); border: 1px solid var(--color-sky-200); border-radius: 12px; \
                        padding: calc(var(--spacing) * 6) calc(var(--spacing) * 7); \
                        box-shadow: 0 2px 12px rgba(0,0,0,.06); \
                        margin-top: calc(var(--spacing) * 6); margin-bottom: calc(var(--spacing) * 6);">
                <form on:submit=on_submit>
                    <div style="display: flex; gap: calc(var(--spacing) * 2.5); flex-wrap: wrap; margin-bottom: calc(var(--spacing) * 5);">
                        <input
                            type="text"
                            placeholder="https://example.com/stream/master.m3u8"
                            style="flex: 1; min-width: 260px; background: var(--color-sky-50); \
                                   border: 1.5px solid var(--color-sky-200); border-radius: 8px; \
                                   color: var(--color-sky-950); font-size: 1rem; \
                                   padding: calc(var(--spacing) * 3) calc(var(--spacing) * 4); outline: none;"
                            prop:value=move || url.get()
                            on:input=move |ev| set_url.set(event_target_value(&ev))
                        />
                        <button
                            type="submit"
                            style="background: linear-gradient(135deg, var(--color-sky-300), var(--color-sky-500)); \
                                   color: var(--color-white); border: none; border-radius: 8px; \
                                   padding: calc(var(--spacing) * 3) calc(var(--spacing) * 7); \
                                   font-size: 1rem; font-weight: 700; cursor: pointer; white-space: nowrap;"
                            disabled=move || loading.get()
                        >
                            {move || if loading.get() { "⏳ Probing…" } else { "🔍 Probe" }}
                        </button>
                    </div>

                    // ── Check category boxes ────────────────────────────────
                    <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); \
                                gap: calc(var(--spacing) * 3);">
                        {CATEGORIES.iter().map(|cat| {
                            let cat_id = cat.id;
                            view! {
                                <div style="background: var(--color-sky-50); border: 1px solid var(--color-sky-200); \
                                            border-radius: 8px; padding: calc(var(--spacing) * 3);">
                                    <div style="display: flex; align-items: center; gap: calc(var(--spacing) * 1.5); \
                                                margin-bottom: calc(var(--spacing) * 2);">
                                        <span style="font-size: .85rem;">{cat.icon}</span>
                                        <span style="font-size: .78rem; font-weight: 700; \
                                                     color: var(--color-sky-950); text-transform: uppercase; \
                                                     letter-spacing: .06em;">
                                            {cat.label}
                                        </span>
                                        <button
                                            type="button"
                                            style="margin-left: auto; font-size: .65rem; font-weight: 600; \
                                                   color: var(--color-sky-300); background: none; border: none; \
                                                   cursor: pointer; padding: 0;"
                                            on:click=move |_| toggle_category(cat_id)
                                        >
                                            {move || {
                                                let cat_items: Vec<&str> = CATEGORIES.iter()
                                                    .find(|c| c.id == cat_id)
                                                    .map(|c| c.items.iter().map(|i| i.id).collect())
                                                    .unwrap_or_default();
                                                let all = cat_items.iter().all(|id| selected.get().contains(*id));
                                                if all { "Deselect all" } else { "Select all" }
                                            }}
                                        </button>
                                    </div>
                                    {cat.items.iter().map(|item| {
                                        let id = item.id.to_string();
                                        let id2 = id.clone();
                                        let label = item.label;
                                        let note = item.note;
                                        view! {
                                            <label style="display: flex; align-items: center; gap: calc(var(--spacing) * 1.5); \
                                                          font-size: .78rem; color: var(--color-sky-800); \
                                                          margin-bottom: var(--spacing); cursor: pointer;">
                                                <input
                                                    type="checkbox"
                                                    prop:checked=move || selected.get().contains(&id)
                                                    on:change=move |_| toggle_check(id2.clone())
                                                />
                                                {label}
                                                {note.map(|n| view! {
                                                    <span style="font-size: .68rem; color: var(--color-sky-200); \
                                                                 font-style: italic;">
                                                        {format!("({})", n)}
                                                    </span>
                                                })}
                                            </label>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                </form>
            </div>

            // ── Loading bar ─────────────────────────────────────────────────
            {move || loading.get().then(|| view! {
                <div style="margin-bottom: calc(var(--spacing) * 4);">
                    <div style="height: 3px; background: var(--color-sky-200); border-radius: 2px; overflow: hidden;">
                        <div style="width: 40%; height: 100%; \
                                    background: linear-gradient(90deg, var(--color-sky-300), var(--color-sky-500)); \
                                    animation: progress 1.5s ease-in-out infinite; border-radius: 2px;">
                        </div>
                    </div>
                    <div style="font-size: .85rem; color: var(--color-sky-700); margin-top: calc(var(--spacing) * 1.5);">
                        "Fetching manifest and probing stream…"
                    </div>
                </div>
            })}

            // ── Error ───────────────────────────────────────────────────────
            {move || error_msg.get().map(|e| view! {
                <div style="padding: calc(var(--spacing) * 3.5) calc(var(--spacing) * 4.5); \
                            background: rgba(239,68,68,.12); \
                            border: 1px solid var(--color-red-400); border-radius: 8px; \
                            color: var(--color-red-400); font-size: .9rem; \
                            margin-bottom: calc(var(--spacing) * 4);">
                    {format!("⚠ {}", e)}
                </div>
            })}

            // ── Results ─────────────────────────────────────────────────────
            {move || report.get().map(|r| view! {
                <ProbeResults report=r selected=selected.get() />
            })}
        </div>
    }
}

// ── Results components ────────────────────────────────────────────────────────

#[component]
fn ProbeResults(report: ProbeReport, selected: HashSet<String>) -> impl IntoView {
    let url = report.url.clone();
    let manifest_type = report.manifest_type.clone();
    let probed_init = report.init_segment_probed;
    let notes = report.probe_notes.clone();

    // Pre-compute all selection booleans before view! to avoid move issues
    let s_format_name   = selected.contains("format_name");
    let s_duration      = selected.contains("duration");
    let s_stream_count  = selected.contains("stream_count");
    let s_session_tags  = selected.contains("session_tags");
    let s_hls_version   = selected.contains("hls_version");
    let s_target_dur    = selected.contains("target_duration");
    let s_playlist_type = selected.contains("playlist_type");
    let s_segment_count = selected.contains("segment_count");
    let s_ll_hls        = selected.contains("ll_hls");
    let s_enc_method    = selected.contains("enc_method");
    let s_key_format    = selected.contains("key_format");
    let s_drm_systems   = selected.contains("drm_systems");
    let s_sub_tracks    = selected.contains("subtitle_tracks");
    let s_sub_lang      = selected.contains("subtitle_lang");
    let s_cap_tracks    = selected.contains("caption_tracks");

    let show_format  = s_format_name || s_duration || s_stream_count || s_session_tags;
    let show_hls     = s_hls_version || s_target_dur || s_playlist_type || s_ll_hls || s_segment_count;
    let show_drm     = s_enc_method || s_key_format || s_drm_systems;
    let show_subs    = s_sub_tracks || s_sub_lang || s_cap_tracks;

    let tags = report.session_tags.clone();
    let ll   = report.ll_hls.clone();
    let enc  = report.encryption_methods.clone();
    let kf   = report.key_formats.clone();
    let drm  = report.drm_systems.clone();
    let subs = report.subtitle_tracks.clone();
    let caps = report.caption_tracks.clone();
    let video_tracks = report.video_tracks.clone();
    let audio_tracks = report.audio_tracks.clone();
    let video_sel = selected.clone();
    let audio_sel = selected;

    view! {
        <div>
            // Header badge
            <div style="display: flex; align-items: flex-start; gap: calc(var(--spacing) * 3); \
                        padding: calc(var(--spacing) * 4) calc(var(--spacing) * 5); \
                        margin-bottom: calc(var(--spacing) * 5); \
                        background: rgba(56,189,248,.08); border: 1px solid rgba(56,189,248,.3); \
                        border-radius: 10px; flex-wrap: wrap;">
                <span style="font-size: 1.4rem; margin-top: 2px;">{"🔍"}</span>
                <div style="flex: 1; min-width: 0;">
                    <div style="font-size: .9rem; font-weight: 700; color: var(--color-sky-500); margin-bottom: calc(var(--spacing) * 2);">{manifest_type}</div>
                    // Multivariant playlist link
                    <div style="margin-bottom: 6px;">
                        <span style="font-size: .72rem; font-weight: 600; color: var(--color-sky-700); text-transform: uppercase; \
                                     letter-spacing: .05em; margin-right: calc(var(--spacing) * 2);">"Multivariant Playlist"</span>
                        <a href={Url::parse(&url).ok().and_then(|b| playlist_href(b, "", &Default::default())).map(|h| format!("/hls-manifest-viewer/{}", h)).unwrap_or_default()}
                           target="_blank"
                           style="font-size: .75rem; color: var(--color-sky-500); word-break: break-all; text-decoration: none; \
                                  border-bottom: 1px dotted var(--color-sky-500);"
                        >{url.clone()}</a>
                    </div>
                    // Video media playlist links (one per variant, ascending by bitrate → pixel count)
                    {(!video_tracks.is_empty()).then(|| {
                        let mut links: Vec<(u64, String, String)> = video_tracks.iter()
                            .filter_map(|vt| vt.init_uri.clone().map(|uri| {
                                // Primary sort key: bitrate; fallback: pixel count from resolution
                                let sort_key = vt.bitrate_bps.unwrap_or_else(|| {
                                    vt.resolution.as_deref()
                                        .and_then(|r| {
                                            let mut pts = r.splitn(2, 'x');
                                            let w = pts.next()?.parse::<u64>().ok()?;
                                            let h = pts.next()?.parse::<u64>().ok()?;
                                            Some(w * h)
                                        })
                                        .unwrap_or(0)
                                });
                                (sort_key, vt.name.clone(), uri)
                            }))
                            .collect();
                        links.sort_by_key(|(k, _, _)| *k);
                        let links: Vec<(String, String)> = links.into_iter().map(|(_, l, u)| (l, u)).collect();
                        (!links.is_empty()).then(|| view! {
                            <div style="margin-bottom: 4px;">
                                <span style="font-size: .72rem; font-weight: 600; color: var(--color-sky-700); text-transform: uppercase; \
                                             letter-spacing: .05em;">"Variant Playlists (init segments)"</span>
                                <div style="display: flex; flex-wrap: wrap; gap: 6px; margin-top: 4px;">
                                    {links.into_iter().map(|(label, uri)| {
                                        let href = Url::parse(&uri).ok().and_then(|b| playlist_href(b, "", &Default::default())).map(|h| format!("/hls-manifest-viewer/{}", h)).unwrap_or_default();
                                        view! {
                                            <a href={href} target="_blank"
                                               style="font-size: .72rem; background: rgba(56,189,248,.12); \
                                                      border: 1px solid rgba(56,189,248,.35); border-radius: 4px; \
                                                      padding: calc(var(--spacing) * 0.5) calc(var(--spacing) * 2); \
                                                      color: var(--color-sky-500); text-decoration: none; white-space: nowrap;">
                                                {label}
                                            </a>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            </div>
                        })
                    })}
                    // Audio media playlist links (one per rendition that has a URI)
                    {(!audio_tracks.is_empty()).then(|| {
                        let links = audio_tracks.iter()
                            .filter_map(|at| at.playlist_uri.clone().map(|uri| (at.name.clone(), uri)))
                            .collect::<Vec<_>>();
                        (!links.is_empty()).then(|| view! {
                            <div>
                                <span style="font-size: .72rem; font-weight: 600; color: var(--color-sky-700); text-transform: uppercase; \
                                             letter-spacing: .05em;">"Audio Playlists"</span>
                                <div style="display: flex; flex-wrap: wrap; gap: 6px; margin-top: 4px;">
                                    {links.into_iter().map(|(label, uri)| {
                                        let href = Url::parse(&uri).ok().and_then(|b| playlist_href(b, "", &Default::default())).map(|h| format!("/hls-manifest-viewer/{}", h)).unwrap_or_default();
                                        view! {
                                            <a href={href} target="_blank"
                                               style="font-size: .72rem; background: rgba(16,185,129,.1); \
                                                      border: 1px solid rgba(16,185,129,.35); border-radius: 4px; \
                                                      padding: calc(var(--spacing) * 0.5) calc(var(--spacing) * 2); \
                                                      color: var(--color-green-600); text-decoration: none; white-space: nowrap;">
                                                {label}
                                            </a>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            </div>
                        })
                    })}
                </div>
                <div style="font-size: .75rem; color: var(--color-sky-700); white-space: nowrap;">
                    {if probed_init { "✓ Init segment fetched" } else { "" }}
                </div>
            </div>

            {(!notes.is_empty()).then(|| {
                let notes2 = notes.clone();
                view! {
                    <div style="margin-bottom: calc(var(--spacing) * 4); \
                                padding: calc(var(--spacing) * 3) calc(var(--spacing) * 4); \
                                background: rgba(245,158,11,.1); border: 1px solid rgba(245,158,11,.35); \
                                border-radius: 8px; font-size: .8rem; color: #d97706;">
                        {notes2.iter().map(|n| view! { <div>{format!("ⓘ {}", n)}</div> }).collect::<Vec<_>>()}
                    </div>
                }
            })}

            // ── Format & Container  +  HLS Protocol  (side by side)
            <div style="display: flex; gap: 20px; align-items: flex-start; flex-wrap: wrap;">
                <div style="flex: 1; min-width: 280px;">
                    <ProbeSection title="📦 Format & Container" show=show_format>
                        <ProbeRow label="Format" value=report.format_name.clone() show=s_format_name />
                        <ProbeRow label="Container (init ftyp)" value=report.major_brand.clone() show=s_format_name />
                        <ProbeRow label="Duration" value=report.duration_s.map(fmt_dur) show=s_duration />
                        <ProbeRow label="Streams" value=Some(report.stream_count.to_string()) show=s_stream_count />
                        {s_session_tags.then(|| {
                            view! {
                                <div>
                                    {(!tags.is_empty()).then(|| view! {
                                        <InfoRow label="Tags / metadata">
                                            <div>
                                                {tags.iter().map(|(k, v)| view! {
                                                    <div style="font-size: .78rem;">
                                                        <b>{k.clone()}</b>{format!(": {}", v)}
                                                    </div>
                                                }).collect::<Vec<_>>()}
                                            </div>
                                        </InfoRow>
                                    })}
                                </div>
                            }
                        })}
                    </ProbeSection>
                </div>
                <div style="flex: 1; min-width: 280px;">
                    <ProbeSection title="📡 HLS Protocol" show=show_hls>
                        <ProbeRow label="HLS version" value=report.hls_version.map(|v| format!("{}", v)) show=s_hls_version />
                        <ProbeRow label="Target duration" value=report.target_duration.map(|d| format!("{} s", d)) show=s_target_dur />
                        <ProbeRow label="Playlist type" value=Some(if report.is_live { "Live (no EXT-X-ENDLIST)".into() } else { report.playlist_type.clone().unwrap_or_else(|| "VOD".into()) }) show=s_playlist_type />
                        <ProbeRow label="Segment count" value=Some(report.total_segments.to_string()) show=s_segment_count />
                        {s_ll_hls.then(|| view! {
                            <div>
                                {if let Some(ll) = ll {
                                    view! {
                                        <InfoRow label="Low-Latency HLS">
                                            <div style="font-size: .78rem;">
                                                {ll.part_hold_back.map(|v| format!("PART-HOLD-BACK={:.3}s ", v)).unwrap_or_default()}
                                                {ll.can_skip_until.map(|v| format!("CAN-SKIP-UNTIL={:.1}s ", v)).unwrap_or_default()}
                                                {if ll.can_block_reload { "CAN-BLOCK-RELOAD=YES" } else { "" }}
                                            </div>
                                        </InfoRow>
                                    }.into_any()
                                } else {
                                    view! { <InfoRow label="Low-Latency HLS"><span>{"Not supported"}</span></InfoRow> }.into_any()
                                }}
                            </div>
                        })}
                    </ProbeSection>
                </div>
            </div>

            // ── Subtitles & Captions  +  Encryption & DRM  (side by side)
            {(show_subs || show_drm).then(|| view! {
                <div style="display: flex; gap: 20px; align-items: flex-start; flex-wrap: wrap;">
                    // Subtitles & Captions
                    <div style="flex: 1; min-width: 280px;">
                        {show_subs.then(|| view! {
                            <div>
                                <SectionTitle label="📝 Subtitles & Captions" />
                                <div style="background: var(--color-sky-50); border: 1px solid var(--color-sky-200); border-radius: 10px; padding: calc(var(--spacing) * 4) calc(var(--spacing) * 4.5); margin-bottom: calc(var(--spacing) * 7);">
                                    {if subs.is_empty() && caps.is_empty() {
                                        view! {
                                            <div style="font-size: .82rem; color: var(--color-sky-700); font-style: italic;">
                                                "No subtitle or caption tracks found in this stream."
                                            </div>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <div>
                                                {(s_sub_tracks && !subs.is_empty()).then(|| view! {
                                                    <div style="margin-bottom: 10px;">
                                                        <div style="font-size: .75rem; font-weight: 700; color: var(--color-sky-700); text-transform: uppercase; margin-bottom: calc(var(--spacing) * 1.5);">"Subtitles"</div>
                                                        {subs.iter().map(|s| view! {
                                                            <div style="font-size: .82rem; color: var(--color-sky-950); margin-bottom: calc(var(--spacing) * 0.75);">
                                                                {format!("{} — lang: {} {}", s.name, s.language.as_deref().unwrap_or("—"), if s.is_default { "(default)" } else { "" })}
                                                            </div>
                                                        }).collect::<Vec<_>>()}
                                                    </div>
                                                })}
                                                {(s_cap_tracks && !caps.is_empty()).then(|| view! {
                                                    <div>
                                                        <div style="font-size: .75rem; font-weight: 700; color: var(--color-sky-700); text-transform: uppercase; margin-bottom: calc(var(--spacing) * 1.5);">"Closed Captions"</div>
                                                        {caps.iter().map(|c| view! {
                                                            <div style="font-size: .82rem; color: var(--color-sky-950); margin-bottom: calc(var(--spacing) * 0.75);">
                                                                {format!("{} (group: {}) — lang: {} {}", c.name, c.group_id, c.language.as_deref().unwrap_or("—"), if c.is_default { "(default)" } else { "" })}
                                                            </div>
                                                        }).collect::<Vec<_>>()}
                                                    </div>
                                                })}
                                            </div>
                                        }.into_any()
                                    }}
                                </div>
                            </div>
                        })}
                    </div>
                    // Encryption & DRM
                    <div style="flex: 1; min-width: 280px;">
                        {show_drm.then(|| view! {
                            <ProbeSection title="🔒 Encryption & DRM" show=true>
                                <ProbeRow label="Encryption methods" value=(!enc.is_empty()).then(|| enc.join(", ")) show=s_enc_method />
                                <ProbeRow label="Key formats" value=(!kf.is_empty()).then(|| kf.join(", ")) show=s_key_format />
                                {s_drm_systems.then(|| view! {
                                    <div>
                                        {if drm.is_empty() {
                                            view! {
                                                <InfoRow label="DRM systems (PSSH)">
                                                    <span style="color: var(--color-sky-700); font-style: italic;">
                                                        {if probed_init { "None found (no PSSH or known KEYFORMAT)" } else { "Init segment not probed" }}
                                                    </span>
                                                </InfoRow>
                                            }.into_any()
                                        } else {
                                            view! {
                                                <InfoRow label="DRM systems (PSSH)">
                                                    <div>
                                                        {drm.iter().map(|d| view! {
                                                            <div style="font-size: .8rem; margin-bottom: calc(var(--spacing) * 0.75);">
                                                                <b>{d.system_name.clone()}</b>
                                                                <span style="color: var(--color-sky-700); font-family: monospace; font-size: .72rem;">
                                                                    {format!(" — {}", d.system_id)}
                                                                </span>
                                                            </div>
                                                        }).collect::<Vec<_>>()}
                                                    </div>
                                                </InfoRow>
                                            }.into_any()
                                        }}
                                    </div>
                                })}
                            </ProbeSection>
                        })}
                    </div>
                </div>
            })}

            // ── Video tracks (table)
            {(!video_tracks.is_empty()).then(|| view! {
                <div>
                    <SectionTitle label="🎬 Video Streams" />
                    <VideoTable tracks=video_tracks selected=video_sel />
                </div>
            })}

            // ── Audio tracks (table)
            {(!audio_tracks.is_empty()).then(|| view! {
                <div>
                    <SectionTitle label="🔊 Audio Streams" />
                    <AudioTable tracks=audio_tracks selected=audio_sel />
                </div>
            })}
        </div>
    }
}

// ── Sub-components ────────────────────────────────────────────────────────────

#[component]
fn SectionTitle(label: &'static str) -> impl IntoView {
    view! {
        <div style="font-size: 1rem; font-weight: 700; color: var(--color-sky-700); text-transform: uppercase; \
                    letter-spacing: .08em; margin-bottom: calc(var(--spacing) * 3); display: flex; align-items: center; gap: calc(var(--spacing) * 2);">
            {label}
            <span style="flex: 1; height: 1px; background: var(--color-sky-200);"></span>
        </div>
    }
}

#[component]
fn ProbeSection(title: &'static str, show: bool, children: Children) -> impl IntoView {
    show.then(|| view! {
        <div style="margin-bottom: calc(var(--spacing) * 7);">
            <SectionTitle label=title />
            <div style="background: var(--color-sky-50); border: 1px solid var(--color-sky-200); border-radius: 10px; padding: calc(var(--spacing) * 4) calc(var(--spacing) * 4.5);">
                {children()}
            </div>
        </div>
    })
}

#[component]
fn ProbeRow(label: &'static str, value: Option<String>, show: bool) -> impl IntoView {
    show.then(|| {
        let display = value.filter(|v| !v.is_empty()).unwrap_or_else(|| "—".into());
        view! {
            <div style="display: flex; gap: calc(var(--spacing) * 3); padding: calc(var(--spacing) * 1.25) 0; \
                        border-bottom: 1px solid var(--color-sky-100); align-items: baseline; flex-wrap: wrap;">
                <span style="min-width: 200px; font-size: .78rem; font-weight: 600; \
                             color: var(--color-sky-700); flex-shrink: 0;">
                    {label}
                </span>
                <span style="font-size: .82rem; color: var(--color-sky-950); font-family: ui-monospace, monospace;">
                    {display}
                </span>
            </div>
        }
    })
}

#[component]
fn InfoRow(label: &'static str, children: Children) -> impl IntoView {
    view! {
        <div style="display: flex; gap: calc(var(--spacing) * 3); padding: calc(var(--spacing) * 1.25) 0; \
                    border-bottom: 1px solid var(--color-sky-100); align-items: baseline; flex-wrap: wrap;">
            <span style="min-width: 200px; font-size: .78rem; font-weight: 600; \
                         color: var(--color-sky-700); flex-shrink: 0;">
                {label}
            </span>
            <div style="font-size: .82rem; color: var(--color-sky-950);">{children()}</div>
        </div>
    }
}

// Column definitions: (check_id, column header) — order matches video_cells() return
const VIDEO_COL_DEFS: &[(&str, &str)] = &[
    ("video_codec",        "Codec"),
    ("video_resolution",   "Resolution"),
    ("video_frame_rate",   "Frame Rate"),
    ("video_bitrate",      "Bitrate"),
    ("video_color_space",  "Color / HDR"),
    ("video_aspect_ratio", "DAR"),
    ("video_profile",      "Profile"),
    ("video_level",        "Level"),
    ("video_bit_depth",    "Bit depth"),
    ("video_primaries",    "Color primaries"),
    ("video_transfer",     "Transfer char."),
    ("video_matrix",       "Matrix coeff."),
    ("video_pixel_fmt",    "Pixel format"),
];

fn video_cells(t: &VideoTrackInfo) -> [Option<String>; 13] {
    [
        t.codec_long.clone().or(t.codec.clone()),
        t.resolution.clone(),
        t.frame_rate.map(|f| format!("{:.3} fps", f)),
        t.bitrate_bps.map(fmt_bps),
        match (&t.color_space, &t.hdr_format) {
            (Some(cs), Some(hdr)) => Some(format!("{} / {}", cs, hdr)),
            (Some(cs), None) => Some(cs.clone()),
            (None, Some(hdr)) => Some(hdr.clone()),
            _ => None,
        },
        match (&t.dar, &t.sar) {
            (Some(dar), Some(sar)) => Some(format!("{} (SAR {})", dar, sar)),
            (Some(dar), None) => Some(dar.clone()),
            _ => None,
        },
        t.profile.clone(),
        t.level.clone(),
        t.bit_depth.map(|b| format!("{}-bit", b)),
        t.color_primaries.clone(),
        t.transfer_characteristics.clone(),
        {
            let mc = t.matrix_coefficients.clone();
            let fr = t.full_range.map(|b| if b { " (full range)" } else { " (limited)" });
            match (mc, fr) {
                (Some(m), Some(f)) => Some(format!("{}{}", m, f)),
                (Some(m), None) => Some(m),
                _ => None,
            }
        },
        t.pixel_format.clone(),
    ]
}

#[component]
fn VideoTable(mut tracks: Vec<VideoTrackInfo>, selected: HashSet<String>) -> impl IntoView {
    // Sort renditions ascending by bitrate
    tracks.sort_by_key(|t| t.bitrate_bps.unwrap_or(0));

    // Determine which columns are visible based on selection
    let vis: Vec<usize> = (0..VIDEO_COL_DEFS.len())
        .filter(|&i| selected.contains(VIDEO_COL_DEFS[i].0))
        .collect();

    let headers: Vec<&'static str> = vis.iter().map(|&i| VIDEO_COL_DEFS[i].1).collect();

    // Pre-compute all cell values: Vec<row> where row = Vec<display_string>
    let table_rows: Vec<Vec<String>> = tracks.iter()
        .map(|t| {
            let cells = video_cells(t);
            vis.iter()
               .map(|&i| cells[i].clone().unwrap_or_else(|| "—".into()))
               .collect()
        })
        .collect();

    view! {
        // Outer wrapper: relative so the scroll-hint overlay is clipped to the table bounds
        <div style="position: relative; margin-bottom: calc(var(--spacing) * 7);">
            <div style="overflow-x: auto; border: 1px solid var(--color-sky-200); border-radius: 10px;">
                <table style="width: 100%; border-collapse: collapse; font-size: .82rem;">
                    <thead>
                        <tr>
                            {headers.into_iter().map(|h| view! {
                                <th style="text-align: left; padding: calc(var(--spacing) * 2.25) calc(var(--spacing) * 3.5); \
                                           background: var(--color-sky-100); \
                                           border-bottom: 2px solid var(--color-sky-200); \
                                           font-size: .7rem; font-weight: 700; color: var(--color-sky-700); \
                                           text-transform: uppercase; letter-spacing: .07em; \
                                           white-space: nowrap;">
                                    {h}
                                </th>
                            }).collect::<Vec<_>>()}
                        </tr>
                    </thead>
                    <tbody>
                        {table_rows.into_iter().enumerate().map(|(ri, row)| {
                            let bg = if ri % 2 == 0 { "var(--color-white)" } else { "var(--color-sky-50)" };
                            view! {
                                <tr style=format!("background: {}; transition: background .1s;", bg)>
                                    {row.into_iter().map(|val| view! {
                                        <td style="padding: calc(var(--spacing) * 2) calc(var(--spacing) * 3.5); \
                                                   border-bottom: 1px solid var(--color-sky-100); \
                                                   color: var(--color-sky-950); \
                                                   font-family: ui-monospace, monospace; \
                                                   white-space: nowrap; vertical-align: middle;">
                                            {val}
                                        </td>
                                    }).collect::<Vec<_>>()}
                                </tr>
                            }
                        }).collect::<Vec<_>>()}
                    </tbody>
                </table>
            </div>
            // Scroll hint: right-edge gradient + animated chevron anchored to the header row.
            // Pinning to the header keeps it visible regardless of how far the user has scrolled.
            // pointer-events: none so it never blocks scroll or click interactions.
            <div style="position: absolute; right: 1px; top: 1px; height: 38px; width: 52px; \
                        border-radius: 0 10px 0 0; pointer-events: none; \
                        background: linear-gradient(to right, transparent, color-mix(in srgb, var(--color-sky-100) 96%, transparent)); \
                        display: flex; align-items: center; justify-content: flex-end; \
                        padding-right: calc(var(--spacing) * 1.75);">
                <style>
                    "@keyframes hls-scroll-bounce {
                        0%, 100% { transform: translateX(0); opacity: .55; }
                        50%       { transform: translateX(4px); opacity: 1; }
                    }"
                </style>
                <span style="font-size: 1.25rem; color: var(--color-sky-700); line-height: 1; \
                             user-select: none; \
                             animation: hls-scroll-bounce 1.4s ease-in-out infinite;">
                    {"›"}
                </span>
            </div>
        </div>
    }
}

// Column definitions for audio table — order matches audio_cells() return
const AUDIO_COL_DEFS: &[(&str, &str)] = &[
    ("audio_codec",    "Codec"),
    ("audio_language", "Language"),
    ("audio_channels", "Channels"),
    ("audio_layout",   "Channel Layout"),
    ("audio_rate",     "Sample Rate"),
    ("audio_depth",    "Bit Depth"),
];

fn audio_cells(t: &AudioTrackInfo) -> [Option<String>; 6] {
    [
        t.codec_long.clone().or(t.codec.clone()),
        t.language.clone(),
        t.channels.map(|c| c.to_string()),
        t.channel_layout.clone(),
        t.sample_rate.map(|r| format!("{} Hz", r)),
        t.bit_depth.map(|b| format!("{}-bit", b)),
    ]
}

#[component]
fn AudioTable(tracks: Vec<AudioTrackInfo>, selected: HashSet<String>) -> impl IntoView {
    let vis: Vec<usize> = (0..AUDIO_COL_DEFS.len())
        .filter(|&i| selected.contains(AUDIO_COL_DEFS[i].0))
        .collect();

    let headers: Vec<&'static str> = vis.iter().map(|&i| AUDIO_COL_DEFS[i].1).collect();

    let table_rows: Vec<(String, bool, Option<u64>, Vec<String>)> = tracks.iter()
        .map(|t| {
            let cells = audio_cells(t);
            let row_cells = vis.iter()
                .map(|&i| cells[i].clone().unwrap_or_else(|| "—".into()))
                .collect();
            (t.name.clone(), t.is_default, t.bitrate_bps, row_cells)
        })
        .collect();

    view! {
        <div style="overflow-x: auto; margin-bottom: calc(var(--spacing) * 7); \
                    border: 1px solid var(--color-sky-200); border-radius: 10px;">
            <table style="width: 100%; border-collapse: collapse; font-size: .82rem;">
                <thead>
                    <tr>
                        <th style="text-align: left; padding: calc(var(--spacing) * 2.25) calc(var(--spacing) * 3.5); \
                                   background: var(--color-sky-100); border-bottom: 2px solid var(--color-sky-200); \
                                   font-size: .7rem; font-weight: 700; color: var(--color-sky-700); \
                                   text-transform: uppercase; letter-spacing: .07em; \
                                   white-space: nowrap;">
                            "Track"
                        </th>
                        {headers.into_iter().map(|h| view! {
                            <th style="text-align: left; padding: calc(var(--spacing) * 2.25) calc(var(--spacing) * 3.5); \
                                       background: var(--color-sky-100); border-bottom: 2px solid var(--color-sky-200); \
                                       font-size: .7rem; font-weight: 700; color: var(--color-sky-700); \
                                       text-transform: uppercase; letter-spacing: .07em; \
                                       white-space: nowrap;">
                                {h}
                            </th>
                        }).collect::<Vec<_>>()}
                    </tr>
                </thead>
                <tbody>
                    {table_rows.into_iter().enumerate().map(|(ri, (name, is_default, bitrate_bps, row))| {
                        let bg = if ri % 2 == 0 { "var(--color-white)" } else { "var(--color-sky-50)" };
                        view! {
                            <tr style=format!("background: {}; transition: background .1s;", bg)>
                                <td style="padding: calc(var(--spacing) * 2) calc(var(--spacing) * 3.5); border-bottom: 1px solid var(--color-sky-100); \
                                           white-space: nowrap; vertical-align: middle;">
                                    <div style="display: flex; align-items: center; gap: 6px;">
                                        <span style="font-size: .82rem; font-weight: 600; color: var(--color-sky-950);">
                                            {name}
                                        </span>
                                        {is_default.then(|| view! {
                                            <span style="font-size: .65rem; color: var(--color-green-600); font-weight: 700; \
                                                         background: rgba(34,197,94,.1); \
                                                         border: 1px solid rgba(34,197,94,.3); \
                                                         border-radius: 4px; padding: var(--spacing) calc(var(--spacing) * 1.25);">
                                                "DEFAULT"
                                            </span>
                                        })}
                                        {bitrate_bps.map(|bps| view! {
                                            <span style="font-size: .72rem; color: var(--color-sky-300);">
                                                {format!("({})", fmt_bps(bps))}
                                            </span>
                                        })}
                                    </div>
                                </td>
                                {row.into_iter().map(|val| view! {
                                    <td style="padding: calc(var(--spacing) * 2) calc(var(--spacing) * 3.5); border-bottom: 1px solid var(--color-sky-100); \
                                               color: var(--color-sky-950); font-family: ui-monospace, monospace; \
                                               white-space: nowrap; vertical-align: middle;">
                                        {val}
                                    </td>
                                }).collect::<Vec<_>>()}
                            </tr>
                        }
                    }).collect::<Vec<_>>()}
                </tbody>
            </table>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPLE_MASTER: &str = "#EXTM3U\n\
        \n\
        #EXT-X-VERSION:6\n\
        \n\
        #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aache-44-64\",NAME=\"English\",LANGUAGE=\"en-US\",AUTOSELECT=YES,DEFAULT=YES,CHANNELS=\"2\",URI=\"audio/prog_index.m3u8\"\n\
        #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"ec3-48-768\",NAME=\"English\",LANGUAGE=\"en-US\",AUTOSELECT=YES,DEFAULT=YES,CHANNELS=\"16/JOC\",URI=\"atmos/prog_index.m3u8\"\n\
        \n\
        #EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subtitles\",NAME=\"English\",LANGUAGE=\"en-US\",AUTOSELECT=YES,FORCED=NO,DEFAULT=YES,URI=\"subs/prog_index.m3u8\"\n\
        \n\
        #EXT-X-MEDIA:TYPE=CLOSED-CAPTIONS,GROUP-ID=\"cc\",LANGUAGE=\"en\",NAME=\"English\",DEFAULT=YES,AUTOSELECT=YES,INSTREAM-ID=\"CC1\"\n\
        \n\
        #EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=2193519,BANDWIDTH=3307941,VIDEO-RANGE=SDR,CODECS=\"avc1.64001f,mp4a.40.5\",RESOLUTION=1024x576,FRAME-RATE=23.976,CLOSED-CAPTIONS=\"cc\",AUDIO=\"aache-44-64\",SUBTITLES=\"subtitles\"\n\
        sdr/prog_index.m3u8\n\
        \n\
        #EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=25097073,BANDWIDTH=37409767,VIDEO-RANGE=PQ,CODECS=\"dvh1.05.06,ec-3\",RESOLUTION=3840x2160,FRAME-RATE=23.976,CLOSED-CAPTIONS=\"cc\",AUDIO=\"ec3-48-768\",SUBTITLES=\"subtitles\"\n\
        dv/prog_index.m3u8\n";

    const APPLE_MEDIA: &str = "#EXTM3U\n\
        #EXT-X-TARGETDURATION:6\n\
        #EXT-X-VERSION:7\n\
        #EXT-X-MEDIA-SEQUENCE:0\n\
        #EXT-X-PLAYLIST-TYPE:VOD\n\
        #EXT-X-INDEPENDENT-SEGMENTS\n\
        \n\
        #EXT-X-PROGRAM-DATE-TIME:2025-01-01T05:00:00.000Z\n\
        #EXT-X-DATERANGE:ID=\"ad1\",CLASS=\"com.apple.hls.interstitial\",START-DATE=\"2025-01-01T05:00:10.000Z\",DURATION=6,X-ASSET-URI=\"https://example.com/ad.m3u8\",CUE=\"PRE\",X-RESUME-OFFSET=0,X-TIMELINE-STYLE=\"HIGHLIGHT\",X-TIMELINE-OCCUPIES=\"POINT\",X-RESTRICT=\"JUMP\",X-PLAYOUT-LIMIT=5.0\n\
        #EXT-X-DATERANGE:ID=\"ad2\",CLASS=\"com.apple.hls.interstitial\",START-DATE=\"2025-01-01T05:00:25.000Z\",DURATION=25,X-ASSET-LIST=\"https://example.com/ads.json\",X-RESUME-OFFSET=0\n\
        \n\
        #EXT-X-MAP:URI=\"fileSequence0.mp4\"\n\
        #EXTINF:5.46379,\t\n\
        fileSequence1.m4s\n\
        #EXTINF:5.21354,\t\n\
        fileSequence2.m4s\n\
        #EXT-X-ENDLIST\n";

    fn count_ok_lines(content: &str) -> (usize, Option<String>) {
        use quick_m3u8::Reader;
        let opts = quick_m3u8::config::ParsingOptions::default();
        let mut reader = Reader::from_str(content, opts);
        let mut n = 0usize;
        loop {
            match reader.read_line() {
                Ok(Some(_)) => n += 1,
                Ok(None) => return (n, None),
                Err(e) => return (n, Some(format!("{e:?}"))),
            }
        }
    }

    #[test]
    fn diagnose_hlsline_kinds() {
        use quick_m3u8::{HlsLine, Reader};
        let content = "#EXTM3U\n\
            #EXT-X-STREAM-INF:BANDWIDTH=1000,RESOLUTION=640x360\n\
            video.m3u8\n";
        let opts = quick_m3u8::config::ParsingOptions::default();
        let mut reader = Reader::from_str(content, opts);
        let mut stream_inf_is_known = false;
        let mut uri_seen = false;
        loop {
            match reader.read_line() {
                Ok(Some(HlsLine::KnownTag(quick_m3u8::tag::KnownTag::Hls(
                    quick_m3u8::tag::hls::Tag::StreamInf(_)
                )))) => { stream_inf_is_known = true; }
                Ok(Some(HlsLine::Uri(_))) => { uri_seen = true; }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(e) => panic!("Err: {e:?}"),
            }
        }
        assert!(stream_inf_is_known, "EXT-X-STREAM-INF must come as KnownTag with default options");
        assert!(uri_seen, "URI line must come as HlsLine::Uri");
    }

    #[test]
    fn diagnose_channels_joc() {
        // CHANNELS="16/JOC" must not break the parser
        let (n, err) = count_ok_lines(
            "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"g\",NAME=\"English\",CHANNELS=\"16/JOC\",DEFAULT=YES\n"
        );
        assert!(err.is_none(), "CHANNELS=16/JOC caused Err after {n} lines: {:?}", err);
    }

    #[test]
    fn diagnose_daterange_tag() {
        // EXT-X-DATERANGE must not break the parser
        let (n, err) = count_ok_lines(
            "#EXTM3U\n#EXT-X-TARGETDURATION:6\n\
             #EXT-X-DATERANGE:ID=\"ad1\",CLASS=\"com.apple.hls.interstitial\",START-DATE=\"2025-01-01T05:00:10.000Z\",DURATION=6,X-ASSET-URI=\"https://example.com/ad.m3u8\",CUE=\"PRE\",X-RESUME-OFFSET=0\n\
             #EXT-X-MAP:URI=\"init.mp4\"\n"
        );
        assert!(err.is_none(), "EXT-X-DATERANGE caused Err after {n} lines: {:?}", err);
        assert_eq!(n, 4, "should read all 4 lines, got {n}");
    }

    #[test]
    fn master_parses_variants_and_renditions() {
        let master = parse_master_playlist("https://example.com/master.m3u8", APPLE_MASTER);
        assert_eq!(master.variants.len(), 2, "should parse 2 EXT-X-STREAM-INF variants");
        assert_eq!(master.version, 6);
        // DVH1 / Dolby Vision variant
        let dv = master.variants.iter().find(|v| v.uri.contains("dv/")).unwrap();
        assert!(dv.codecs.as_deref().unwrap().starts_with("dvh1"), "dvh1 codec must be preserved");
        // Audio renditions: 2 (one JOC, one stereo)
        let audio: Vec<_> = master.media_renditions.iter().filter(|r| r.media_type == "AUDIO").collect();
        assert_eq!(audio.len(), 2);
        // CHANNELS="16/JOC" should not cause a parse break
        let joc = audio.iter().find(|r| r.name.contains("English") && r.group_id == "ec3-48-768");
        assert!(joc.is_some(), "JOC audio rendition should be present");
        // Subtitles
        let subs: Vec<_> = master.media_renditions.iter().filter(|r| r.media_type == "SUBTITLES").collect();
        assert_eq!(subs.len(), 1);
        // Closed captions (no URI)
        let cc: Vec<_> = master.media_renditions.iter().filter(|r| r.media_type == "CLOSED-CAPTIONS").collect();
        assert_eq!(cc.len(), 1);
    }

    #[test]
    fn media_playlist_parsed_past_daterange() {
        let pl = parse_media_playlist("https://example.com/sdr/prog_index.m3u8", APPLE_MEDIA);
        assert_eq!(pl.target_duration, 6.0, "EXT-X-TARGETDURATION should be 6");
        assert_eq!(pl.playlist_type.as_deref(), Some("VOD"), "playlist type should be VOD");
        assert!(pl.has_endlist, "EXT-X-ENDLIST should be detected");
        assert_eq!(pl.segments.len(), 2, "should parse 2 segments (EXT-X-DATERANGE must not break the loop)");
        // EXT-X-MAP URI must be propagated to each segment
        assert!(
            pl.segments.iter().all(|s| s.map_uri.as_deref() == Some("https://example.com/sdr/fileSequence0.mp4")),
            "all segments must carry the resolved map_uri"
        );
    }
}
