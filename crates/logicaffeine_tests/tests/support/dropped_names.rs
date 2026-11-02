//! The names a file can be dropped by and the type a browser gives each — the tables
//! `dropped_file_type_lock` holds Chrome to and `browser_mime_type_lock` holds the tiers' shared
//! mapping to, so the two locks cannot pin different browsers.
#![allow(dead_code)]

/// Every name whose type the browser's own table settles, with that type: every extension of
/// Chromium's primary list (which no desktop overrides), the secondary-list extensions this
/// desktop's own database agrees on or does not name, an extension no browser knows, no extension
/// at all, a name that is only an extension, upper-case spellings, a name with several dots and a
/// trailing dot.
pub const DROPS: &[(&str, &str)] = &[
    // The drive sentence `docs/frontend.md` teaches, and the shapes around it.
    ("notes.txt", "text/plain"),
    ("img.png", "image/png"),
    ("page.html", "text/html"),
    ("data.csv", "text/csv"),
    ("archive.zip", "application/zip"),
    ("a.pdf", "application/pdf"),
    ("readme.md", "text/markdown"),
    ("noext", ""),
    ("weird.qqq", ""),
    ("PHOTO.JPG", "image/jpeg"),
    ("dots.in.name.txt", "text/plain"),
    (".hidden", ""),
    (".png", "image/png"),
    ("trailing.", ""),
    // Every extension of the browser's primary list; the first entry naming an extension wins.
    ("a.webm", "video/webm"),
    ("a.mp3", "audio/mpeg"),
    ("a.wasm", "application/wasm"),
    ("a.crx", "application/x-chrome-extension"),
    ("a.xhtml", "application/xhtml+xml"),
    ("a.xht", "application/xhtml+xml"),
    ("a.xhtm", "application/xhtml+xml"),
    ("a.flac", "audio/flac"),
    ("a.mka", "audio/matroska"),
    ("a.ogg", "audio/ogg"),
    ("a.oga", "audio/ogg"),
    ("a.opus", "audio/ogg"),
    ("a.wav", "audio/wav"),
    ("a.m4a", "audio/x-m4a"),
    ("a.avif", "image/avif"),
    ("a.gif", "image/gif"),
    ("a.jpeg", "image/jpeg"),
    ("a.jpg", "image/jpeg"),
    ("a.jpe", "image/jpeg"),
    ("a.jxl", "image/jxl"),
    ("a.apng", "image/apng"),
    ("a.svg", "image/svg+xml"),
    ("a.svgz", "image/svg+xml"),
    ("a.webp", "image/webp"),
    ("a.mht", "multipart/related"),
    ("a.mhtml", "multipart/related"),
    ("a.css", "text/css"),
    ("a.htm", "text/html"),
    ("a.shtml", "text/html"),
    ("a.shtm", "text/html"),
    ("a.js", "text/javascript"),
    ("a.mjs", "text/javascript"),
    ("a.xml", "text/xml"),
    ("a.mkv", "video/matroska"),
    ("a.mp4", "video/mp4"),
    ("a.m4v", "video/mp4"),
    ("a.ogv", "video/ogg"),
    ("a.ogm", "video/ogg"),
    // The browser's secondary list, where this desktop's database agrees with it or is silent.
    ("a.epub", "application/epub+zip"),
    ("bundle.tar.gz", "application/gzip"),
    ("a.doc", "application/msword"),
    ("a.bin", "application/octet-stream"),
    ("a.com", "application/octet-stream"),
    ("a.p7m", "application/pkcs7-mime"),
    ("a.p7c", "application/pkcs7-mime"),
    ("a.p7z", "application/pkcs7-mime"),
    ("a.p7s", "application/pkcs7-signature"),
    ("a.ps", "application/postscript"),
    ("a.rdf", "application/rdf+xml"),
    ("a.rss", "application/rss+xml"),
    ("a.rtf", "application/rtf"),
    ("a.apk", "application/vnd.android.package-archive"),
    ("a.xul", "application/vnd.mozilla.xul+xml"),
    ("a.xls", "application/vnd.ms-excel"),
    ("a.ppt", "application/vnd.ms-powerpoint"),
    (
        "a.pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
    (
        "a.xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    (
        "a.docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    ("a.swl", "application/x-shockwave-flash"),
    ("a.tar", "application/x-tar"),
    ("a.crt", "application/x-x509-ca-cert"),
    ("a.weba", "audio/webm"),
    ("a.bmp", "image/bmp"),
    ("a.jfif", "image/jpeg"),
    ("a.pjpeg", "image/jpeg"),
    ("a.pjp", "image/jpeg"),
    ("a.tiff", "image/tiff"),
    ("a.tif", "image/tiff"),
    ("a.xbm", "image/x-xbitmap"),
    ("a.eml", "message/rfc822"),
    ("a.ics", "text/calendar"),
    ("a.ehtml", "text/html"),
    ("a.text", "text/plain"),
    ("a.vtt", "text/vtt"),
    ("a.mpeg", "video/mpeg"),
    ("a.mpg", "video/mpeg"),
    ("a.mpe", "video/mpeg"),
];

/// Names whose extension the browser's primary list does not name but a desktop's database may:
/// a browser asks the desktop first and reads its secondary list only where the desktop is silent.
/// The type written here is the table's, what the in-process tiers answer on every machine; a
/// real browser answers with its desktop's type where the desktop has one ([`desktop_types`]).
pub const DESKTOP_DROPS: &[(&str, &str)] = &[
    ("script.sh", "text/x-sh"),
    ("icon.ico", "image/x-icon"),
    ("font.woff", "application/font-woff"),
    ("bundle.tgz", "application/gzip"),
    ("figure.eps", "application/postscript"),
    ("art.ai", "application/postscript"),
    ("setup.exe", "application/octet-stream"),
    ("data.json", "application/json"),
    ("template.dot", "application/msword"),
    ("stream.m3u8", "application/x-mpegurl"),
    ("movie.swf", "application/x-shockwave-flash"),
    ("cert.cer", "application/x-x509-ca-cert"),
    ("sheet.xsl", "text/xml"),
    ("sheet.xslt", "text/xml"),
    ("binding.xbl", "text/xml"),
];

/// The types this desktop's own MIME database names the extension of `name` by: the shared MIME
/// database's `globs2` tables under `$XDG_DATA_HOME/mime` and each of `$XDG_DATA_DIRS/mime`, the
/// tables a browser on Linux asks between its two lists, read as the browser reads their answer
/// (`application/octet-stream` counts as none, and `image/x-ico` is the browser's `image/x-icon`).
/// Empty where no table names the extension, and on a desktop without the database — every other
/// operating system, and a bare container.
pub fn desktop_types(name: &str) -> Vec<String> {
    let Some(ext) = name
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| !ext.is_empty())
    else {
        return Vec::new();
    };
    let data_home = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|dir| !dir.is_empty())
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|home| format!("{home}/.local/share"))
        });
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    let mut types: Vec<String> = Vec::new();
    for dir in data_home
        .into_iter()
        .chain(data_dirs.split(':').map(str::to_string))
    {
        let Ok(globs) = std::fs::read_to_string(format!("{dir}/mime/globs2")) else {
            continue;
        };
        for line in globs.lines().filter(|line| !line.starts_with('#')) {
            let mut fields = line.split(':');
            let (Some(_weight), Some(mime), Some(pattern)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let case_sensitive = fields.next() == Some("cs");
            let Some(known) = pattern.strip_prefix("*.") else {
                continue;
            };
            let names_it = if case_sensitive {
                known == ext
            } else {
                known.eq_ignore_ascii_case(ext)
            };
            if !names_it || mime == "application/octet-stream" {
                continue;
            }
            let mime = if mime == "image/x-ico" {
                "image/x-icon"
            } else {
                mime
            };
            if !types.iter().any(|t| t == mime) {
                types.push(mime.to_string());
            }
        }
    }
    types
}
