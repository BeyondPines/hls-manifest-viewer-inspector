# hls-manifest-viewer

This is a static site written almost entirely in Rust using the [Leptos][1] framework (compiled to
WebAssembly for serving). It makes use of my [m3u8][2] parsing library (also written in Rust).

The purpose of this site is to provide a tool that helps make browsing HLS manifests easier.
Sometimes, as a video streaming developer, you need to investigate issues that a player is having
and it can help to inspect the media (both playlists and media segments) it is receiving. With HLS
this can be an arduous process since the media segment locations are usually described using
multiple manifest files (one multivariant playlist and many media playlists). It can be frustrating
to need to download the multivariant, figure out the absolute URLs for the media playlists, and then
also download those. Add into the mix [HLS Variable Substitution][3] and this can get very boring
very quickly. This tools hopes to put everything needed to browse HLS media in a convenient user
interface that can be loaded in your web browser. Here are some of the goals of the tool:
- [x] Provide resolved navigation links for media playlist URIs in the HLS playlist that don't
      require leaving the tool.
- [ ] Provide a view for inspecting [Media Segments][4] within the tool.
    * HLS has support for [MPEG-2 Transport Streams][5], [Fragmented MPEG-4][6], [Packed Audio][7],
      [WebVTT][8], and [IMSC Subtitles][9]. The highest priority for this tool is Fragmented MPEG-4
      support (mainly because it is what I deal with mostly, but also, it seems to be the defacto
      standard for streaming these days thanks to CMAF). WebVTT has basic support (given it is just
      text it is easy to dump that into a view). IMSC Subtitles can be seen as _partially_ supported
      since they are contained within Fragmented MPEG-4; however, there is a goal to provide better
      views into fMP4 contents (perhaps a view on CEA-608/708 captions, `id3` parsing within `emsg`
      atoms, etc.).
    - [ ] MPEG-2 Transport Streams
    - [x] Fragmented MPEG-4
    - [ ] Packed Audio
    - [x] WebVTT
    - [ ] IMSC Subtitles
- [x] Provide a view for parsing SCTE35 messages found in `EXT-X-DATERANGE` tags.
- [ ] Provide a view for visualizing what range of segments a given `EXT-X-DATERANGE` tag applies to
      (given that `EXT-X-DATERANGE` is described using [ISO_8601][10] and can appear anywhere in the
      playlist it isn't always easy to see if a segment is included in the range or not).
- [ ] Provide a view for JSON links from the manifest (such as for [Content Steering][11], for the
      [X-ASSET-LIST][12] attribute in HLS Interstitials, [EXT-X-SESSION-DATA][13], etc.).

So far only media playlist resolution, fMP4 (including range requests based on `EXT-X-BYTERANGE` /
`EXT-X-MAP:BYTERANGE`), WebVTT, and SCTE35 views have been implemented.

But ultimately, this tool is just meant to be helpful in making working with HLS easier as a player
developer, and so there may be many other fun directions to go in. For example, an MSE player view
to see frames of content within a chosen Media Segment, or perhaps WebCodecs, or a validation tool
for the playlist (like [mediastreamvalidator][14])... Lots of ways to spend time here.

Also, I'm only working on this in my spare time, so a lot of these ideas may not get done if just
left to me. Based on this, I provide this with the [Unlicense license][15] so anyone is free to take
it and do what they want with it. If I'm really honest, I made this because I don't get to code much
these days but when I do I really like Rust, so I was searching for reasons to write something.
That's why this is written using Leptos rather than a more normal web framework (e.g. React, Vue, or
just plain old HTML+CSS+JavaScript).

[1]: https://leptos.dev
[2]: https://github.com/theRealRobG/m3u8
[3]: https://datatracker.ietf.org/doc/html/draft-pantos-hls-rfc8216bis
[4]: https://developer.apple.com/documentation/http-live-streaming/using-apple-s-http-live-streaming-hls-tools
[5]: https://unlicense.org/

## Building Locally

### Prerequisites
* Rust (https://www.rust-lang.org/tools/install)
* `wasm32-unknown-unknown` target (https://doc.rust-lang.org/beta/rustc/platform-support/wasm32-unknown-unknown.html#building-rust-programs)
* Trunk (https://trunkrs.dev/#plain-cargo)

### Serve
In the root of the repository run:
```
trunk serve --public-url "/hls-manifest-viewer"
```
Annoyingly the `--public-url` flag is needed because I've hard-coded the GitHub Pages site path into
the project source code. GitHub Pages publishes the site to a path component from the base domain
(https://therealrobg.github.io/hls-manifest-viewer in this case). Trunk can fix source locations for
this by using this flag, which would hopefully only be needed when publishing; however, the Leptos
Router is broken by this unexpected path component. I've experimented with making this more dynamic,
as you can read the site's [Base URI][16] (from the Document), so theoretically I could update the
router paths for each page to be more dynamic on startup. I had a brief exploration here but the
paths defined in the router need to have static lifetimes so it's difficult to derive a path at
runtime (I _could_ do some nasty things like using [String::leak][17], but in the end, it was late
and I just wanted to get the site to work). This should be improved.

[16]: https://developer.mozilla.org/en-US/docs/Web/API/Node/baseURI
[17]: https://doc.rust-lang.org/std/string/struct.String.html#method.leak

### Release
The release process is handled by GitHub Actions ([pages.yml](.github/workflows/pages.yml)). But if
you want to build for release locally then run the following command:
```
trunk build --release --public-url "/hls-manifest-viewer"
```


[18]: https://chromewebstore.google.com/detail/adaptive-bitrate-manifest/omjpjjekjefmdkidigpkhpjnojoadbih?hl=en
[19]: https://dist.gpac.io/mp4box.js/test/filereader.html
[20]: https://github.com/Comcast/mamba
[21]: https://emarsden.github.io/pssh-box-wasm/
