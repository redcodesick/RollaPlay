use std::path::{Component, Path, PathBuf};
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaylistFileKind {
    M3u,
    M3u8,
    Wpl,
}

impl PlaylistFileKind {
    pub fn from_extension(extension: &str) -> Option<Self> {
        if extension.eq_ignore_ascii_case("m3u") {
            Some(Self::M3u)
        } else if extension.eq_ignore_ascii_case("m3u8") {
            Some(Self::M3u8)
        } else if extension.eq_ignore_ascii_case("wpl") {
            Some(Self::Wpl)
        } else {
            None
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::M3u => "m3u",
            Self::M3u8 => "m3u8",
            Self::Wpl => "wpl",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PlaylistFileEntry {
    pub reference: String,
    pub title: Option<String>,
    pub duration_seconds: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct ParsedPlaylistFile {
    pub name: Option<String>,
    pub entries: Vec<PlaylistFileEntry>,
}

#[derive(Clone, Debug)]
pub struct ExportItem {
    pub path: PathBuf,
    pub filename: String,
    pub duration_seconds: Option<f64>,
}

fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let haystack = haystack.as_bytes().to_ascii_lowercase();
    let needle = needle.as_bytes().to_ascii_lowercase();
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle.as_slice())
}

fn find_ci_from(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    if from > haystack.len() {
        return None;
    }
    find_ci(&haystack[from..], needle).map(|offset| from + offset)
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String, String> {
    if bytes.len() % 2 != 0 {
        return Err("Playlist file has invalid Unicode data".to_string());
    }
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| {
            if little_endian {
                u16::from_le_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_be_bytes([chunk[0], chunk[1]])
            }
        })
        .collect::<Vec<_>>();
    let mut text = String::from_utf16(&units)
        .map_err(|_| "Playlist file has invalid Unicode data".to_string())?;
    if text.starts_with('\u{feff}') {
        text.remove(0);
    }
    Ok(text)
}

pub fn decode_playlist_text(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(&[0xff, 0xfe]) {
        return decode_utf16(&bytes[2..], true);
    }
    if bytes.starts_with(&[0xfe, 0xff]) {
        return decode_utf16(&bytes[2..], false);
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

pub fn parse_m3u(text: &str, fallback_name: &str) -> ParsedPlaylistFile {
    let mut name = None;
    let mut pending: Option<(Option<f64>, Option<String>)> = None;
    let mut entries = Vec::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(comment) = line.strip_prefix('#') {
            let upper = comment.to_ascii_uppercase();
            if upper.starts_with("EXTINF:") {
                let original = &comment[7..];
                let (duration_text, title) = match original.split_once(',') {
                    Some((duration, title)) => (duration.trim(), Some(title.trim().to_string())),
                    None => (original.trim(), None),
                };
                let duration = duration_text
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite() && *value > 0.0);
                let title = title.filter(|value| !value.is_empty());
                pending = Some((duration, title));
            } else if upper.starts_with("PLAYLIST:") {
                let title = comment[9..].trim().to_string();
                if !title.is_empty() {
                    name = Some(title);
                }
            }
            continue;
        }
        entries.push(PlaylistFileEntry {
            reference: line.to_string(),
            title: pending.as_ref().and_then(|(_, title)| title.clone()),
            duration_seconds: pending.as_ref().and_then(|(duration, _)| *duration),
        });
        pending = None;
    }

    ParsedPlaylistFile {
        name: name.or_else(|| {
            let trimmed = fallback_name.trim();
            if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
        }),
        entries,
    }
}

pub fn unescape_xml(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find('&') {
        result.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find(';') else {
            result.push('&');
            rest = after;
            continue;
        };
        let entity = &after[..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16).ok().and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => entity[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        if let Some(character) = decoded {
            result.push(character);
        } else {
            result.push('&');
            result.push_str(entity);
            result.push(';');
        }
        rest = &after[end + 1..];
    }
    result.push_str(rest);
    result
}

fn escape_xml(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&apos;"),
            _ => result.push(character),
        }
    }
    result
}

fn xml_tags(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut tags = Vec::new();
    let mut index = 0;
    let mut start = 0;
    let mut in_tag = false;
    let mut quote: Option<u8> = None;
    while index < bytes.len() {
        let byte = bytes[index];
        if !in_tag {
            if byte == b'<' {
                in_tag = true;
                start = index;
            }
        } else if let Some(current) = quote {
            if byte == current {
                quote = None;
            }
        } else if byte == b'"' || byte == b'\'' {
            quote = Some(byte);
        } else if byte == b'>' {
            tags.push(text[start..=index].to_string());
            in_tag = false;
        }
        index += 1;
    }
    tags
}

fn tag_local_name(tag: &str) -> Option<&str> {
    let inner = tag.strip_prefix('<')?.trim_end_matches('>').trim();
    let inner = inner.strip_prefix('/').unwrap_or(inner).trim();
    if inner.is_empty() || inner.starts_with('?') || inner.starts_with('!') {
        return None;
    }
    let end = inner
        .find(|character: char| character.is_whitespace() || character == '/' || character == '>')
        .unwrap_or(inner.len());
    let name = &inner[..end];
    Some(name.rsplit(':').next().unwrap_or(name))
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let pattern = format!("{}=", name);
    let lower = tag.to_ascii_lowercase();
    let bytes = tag.as_bytes();
    let mut from = 0;
    while let Some(position) = find_ci_from(&lower, &pattern, from) {
        let boundary = if position == 0 {
            true
        } else {
            let previous = bytes[position - 1];
            !(previous.is_ascii_alphanumeric()
                || previous == b'_'
                || previous == b':'
                || previous == b'-')
        };
        if !boundary {
            from = position + 1;
            continue;
        }
        let mut index = position + pattern.len();
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let quote = *bytes.get(index)?;
        if quote != b'"' && quote != b'\'' {
            return None;
        }
        index += 1;
        let value_start = index;
        while index < bytes.len() && bytes[index] != quote {
            index += 1;
        }
        if index >= bytes.len() {
            return None;
        }
        return Some(unescape_xml(&tag[value_start..index]));
    }
    None
}

pub fn parse_wpl(text: &str, fallback_name: &str) -> ParsedPlaylistFile {
    let mut name = None;
    if let Some(start) = find_ci(text, "<title") {
        if let Some(greater_than) = text[start..].find('>') {
            let content_start = start + greater_than + 1;
            if let Some(close) = find_ci_from(text, "</title>", content_start) {
                let title = unescape_xml(text[content_start..close].trim());
                if !title.is_empty() {
                    name = Some(title);
                }
            }
        }
    }

    let mut entries = Vec::new();
    for tag in xml_tags(text) {
        let Some(local_name) = tag_local_name(&tag) else { continue };
        if !local_name.eq_ignore_ascii_case("media") {
            continue;
        }
        if let Some(source) = attribute(&tag, "src") {
            if !source.trim().is_empty() {
                entries.push(PlaylistFileEntry {
                    reference: source,
                    title: None,
                    duration_seconds: None,
                });
            }
        }
    }

    ParsedPlaylistFile {
        name: name.or_else(|| {
            let trimmed = fallback_name.trim();
            if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
        }),
        entries,
    }
}

pub fn parse_playlist_file(path: &Path) -> Result<ParsedPlaylistFile, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let kind = PlaylistFileKind::from_extension(extension)
        .ok_or_else(|| "Unsupported playlist format".to_string())?;
    let bytes = fs_read(path)?;
    let text = decode_playlist_text(&bytes)?;
    let fallback = path
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Imported playlist".to_string());
    Ok(match kind {
        PlaylistFileKind::M3u | PlaylistFileKind::M3u8 => parse_m3u(&text, &fallback),
        PlaylistFileKind::Wpl => parse_wpl(&text, &fallback),
    })
}

fn fs_read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|_| "Could not read playlist file".to_string())
}

/// Returns `None` for remote streams, so imported entries can never become
/// Cast URLs. Only local paths and `file://` URIs are accepted.
pub fn reference_to_local_path(reference: &str) -> Option<PathBuf> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains("://") {
        let url = Url::parse(trimmed).ok()?;
        if url.scheme().eq_ignore_ascii_case("file") {
            return url.to_file_path().ok();
        }
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if ["http:", "https:", "rtsp:", "rtmp:", "mms:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
    {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

fn single_line(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character != '\r' && *character != '\n')
        .collect()
}

fn component_matches(left: &Component<'_>, right: &Component<'_>) -> bool {
    if cfg!(windows) {
        left.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
    } else {
        left.as_os_str() == right.as_os_str()
    }
}

fn relative_reference(output_directory: &Path, source: &Path) -> String {
    let base: Vec<Component<'_>> = output_directory.components().collect();
    let target: Vec<Component<'_>> = source.components().collect();
    let mut common = 0;
    while common < base.len() && common < target.len() && component_matches(&base[common], &target[common]) {
        common += 1;
    }
    if common == 0 {
        return source.to_string_lossy().into_owned();
    }
    let mut parts = Vec::new();
    for _ in common..base.len() {
        parts.push("..".to_string());
    }
    for component in &target[common..] {
        parts.push(component.as_os_str().to_string_lossy().into_owned());
    }
    if parts.is_empty() {
        parts.push("media".to_string());
    }
    parts.join("/")
}

pub fn export_m3u(name: &str, items: &[ExportItem], output_directory: &Path) -> String {
    let mut lines = vec![
        "#EXTM3U".to_string(),
        format!("#PLAYLIST:{}", single_line(name)),
    ];
    for item in items {
        if let Some(duration) = item
            .duration_seconds
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            lines.push(format!(
                "#EXTINF:{},{}",
                duration.round() as i64,
                single_line(&item.filename)
            ));
        }
        lines.push(relative_reference(output_directory, &item.path));
    }
    format!("{}\r\n", lines.join("\r\n"))
}

pub fn export_wpl(name: &str, items: &[ExportItem], output_directory: &Path) -> String {
    let mut xml = String::from("<?wpl version=\"1.0\"?>\r\n<smil>\r\n  <head>\r\n    <title>");
    xml.push_str(&escape_xml(&single_line(name)));
    xml.push_str("</title>\r\n  </head>\r\n  <body>\r\n    <seq>\r\n");
    for item in items {
        xml.push_str("      <media src=\"");
        xml.push_str(&escape_xml(&relative_reference(output_directory, &item.path)));
        xml.push_str("\" />\r\n");
    }
    xml.push_str("    </seq>\r\n  </body>\r\n</smil>\r\n");
    xml
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m3u_parses_extinf_titles_and_ignores_comments() {
        let parsed = parse_m3u(
            "#EXTM3U\r\n#PLAYLIST:Road Trip\r\n#EXTINF:123,Song & One\r\nsongs/one.mp4\r\n#EXTINF:-1,Fallback\r\nfile:///C:/media/two.mp4\r\n#EXTVLCOPT:no-video\r\n",
            "ignored",
        );
        assert_eq!(parsed.name.as_deref(), Some("Road Trip"));
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].reference, "songs/one.mp4");
        assert_eq!(parsed.entries[0].title.as_deref(), Some("Song & One"));
        assert_eq!(parsed.entries[0].duration_seconds, Some(123.0));
        assert!(parsed.entries[1].duration_seconds.is_none());
        assert_eq!(parsed.entries[1].reference, "file:///C:/media/two.mp4");
    }

    #[test]
    fn utf8_bom_is_removed_before_parsing() {
        let bytes = b"\xef\xbb\xbf#EXTM3U\nclip.mp4\n";
        let text = decode_playlist_text(bytes).unwrap();
        assert!(text.starts_with("#EXTM3U"));
        let parsed = parse_m3u(&text, "Fallback");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.name.as_deref(), Some("Fallback"));
    }

    #[test]
    fn wpl_parses_namespaced_media_and_escaped_paths() {
        let text = "<?wpl version=\"1.0\"?><smil xmlns=\"http://schemas.microsoft.com/wpl\">\
            <head><title>Rock &amp; Roll</title></head><body><seq>\
            <wpl:media src=\"videos/a &amp; b.mp4\"/><media src='C:\\media\\c.mp4'/>\
            </seq></body></smil>";
        let parsed = parse_wpl(text, "fallback");
        assert_eq!(parsed.name.as_deref(), Some("Rock & Roll"));
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].reference, "videos/a & b.mp4");
        assert_eq!(parsed.entries[1].reference, r"C:\media\c.mp4");
    }

    #[test]
    fn utf16_wpl_decodes_to_unicode() {
        let source = "<?wpl version=\"1.0\"?><smil><head><title>Été</title></head>\
            <body><seq><media src=\"été.mp4\"/></seq></body></smil>";
        let units: Vec<u16> = source.encode_utf16().collect();
        let mut bytes = vec![0xff, 0xfe];
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let text = decode_playlist_text(&bytes).unwrap();
        let parsed = parse_wpl(&text, "fallback");
        assert_eq!(parsed.name.as_deref(), Some("Été"));
        assert_eq!(parsed.entries[0].reference, "été.mp4");
    }

    #[test]
    fn remote_references_are_not_local_paths() {
        assert!(reference_to_local_path("http://example.com/stream.m3u8").is_none());
        assert!(reference_to_local_path("rtsp://camera/live").is_none());
        assert!(reference_to_local_path("file:///C:/media/movie.mp4").is_some());
        assert!(reference_to_local_path("videos/movie.mp4").is_some());
    }

    #[test]
    fn export_m3u_uses_relative_paths_and_single_line_titles() {
        let items = [ExportItem {
            path: PathBuf::from("media/videos/one.mp4"),
            filename: "one.mp4".to_string(),
            duration_seconds: Some(12.6),
        }];
        let output = export_m3u("My\nPlaylist", &items, Path::new("media/playlists"));
        assert!(output.starts_with("#EXTM3U\r\n#PLAYLIST:MyPlaylist\r\n"));
        assert!(output.contains("#EXTINF:13,one.mp4\r\n../videos/one.mp4"));
    }
}
