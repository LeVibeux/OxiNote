//! Image embeds shown under their line in Live Preview: detection of lines
//! that hold only an image (`![[img.png|300]]` or `![alt](path)`) and sizing.

/// Extensions drawn inline (the image loaders of the app decode these).
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// An image-only line.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRef {
    /// Link target as written (decoded for markdown links, no `#`/`|`).
    pub target: String,
    /// Requested size (`|300` or `|300x200`).
    pub width: Option<f32>,
    pub height: Option<f32>,
}

fn has_image_ext(target: &str) -> bool {
    let name = target.rsplit(['/', '\\']).next().unwrap_or(target);
    name.rsplit_once('.')
        .is_some_and(|(_, e)| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// `300` or `300x200` (also used by `|alt|300`).
fn parse_size(s: &str) -> Option<(f32, Option<f32>)> {
    let s = s.trim();
    let (w, h) = match s.split_once(['x', 'X']) {
        Some((w, h)) => (w, Some(h)),
        None => (s, None),
    };
    let w: f32 = w.trim().parse().ok().filter(|v| *v > 0.0)?;
    let h = match h {
        Some(h) => Some(h.trim().parse::<f32>().ok().filter(|v| *v > 0.0)?),
        None => None,
    };
    Some((w, h))
}

/// Percent-decode a markdown destination (invalid sequences kept as is).
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// If `line` holds nothing but one local image embed, describe it.
pub fn image_line(line: &str) -> Option<ImageRef> {
    let t = line.trim();
    if let Some(inner) = t.strip_prefix("![[").and_then(|r| r.strip_suffix("]]")) {
        if inner.contains("]]") || inner.contains("[[") {
            return None;
        }
        let mut parts = inner.split('|');
        let target = parts.next()?.trim();
        let target = target.split('#').next().unwrap_or(target).trim();
        if target.is_empty() || !has_image_ext(target) {
            return None;
        }
        // The size is the last numeric part (`![[a.png|alt|300]]`).
        let size = parts.filter_map(parse_size).last();
        return Some(ImageRef {
            target: target.to_string(),
            width: size.map(|s| s.0),
            height: size.and_then(|s| s.1),
        });
    }
    let rest = t.strip_prefix("![")?;
    let close = rest.find("](")?;
    let alt = &rest[..close];
    let dest = rest[close + 2..].strip_suffix(')')?;
    if dest.contains(')') && !dest.starts_with('<') {
        return None;
    }
    let dest = dest.trim();
    let dest = match dest.strip_prefix('<') {
        Some(d) => d.strip_suffix('>')?,
        // Optional title: `path "title"`.
        None => dest.split(" \"").next().unwrap_or(dest),
    };
    if dest.is_empty() || dest.contains("://") || dest.starts_with("data:") {
        return None;
    }
    let target = percent_decode(dest);
    let target = target.split('#').next().unwrap_or(&target).to_string();
    if !has_image_ext(&target) {
        return None;
    }
    let size = alt.rsplit('|').next().and_then(parse_size);
    Some(ImageRef {
        target,
        width: size.map(|s| s.0),
        height: size.and_then(|s| s.1),
    })
}

/// Maximum height of an embedded image.
pub const MAX_HEIGHT: f32 = 600.0;

/// Display size of an image of `natural` pixels in a column `max_w` wide:
/// the requested width (never above the column), the natural size
/// otherwise, always keeping the aspect ratio when only one side is given.
pub fn display_size(natural: (u32, u32), r: &ImageRef, max_w: f32) -> (f32, f32) {
    let (nw, nh) = (natural.0.max(1) as f32, natural.1.max(1) as f32);
    let (mut w, mut h) = match (r.width, r.height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w * nh / nw),
        _ => (nw, nh),
    };
    let max_w = max_w.max(32.0);
    if w > max_w {
        h *= max_w / w;
        w = max_w;
    }
    if h > MAX_HEIGHT {
        w *= MAX_HEIGHT / h;
        h = MAX_HEIGHT;
    }
    (w.max(1.0), h.max(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(t: &str) -> Option<ImageRef> {
        image_line(t)
    }

    #[test]
    fn wiki_images() {
        let r = img("![[schema.png]]").unwrap();
        assert_eq!(r.target, "schema.png");
        assert_eq!((r.width, r.height), (None, None));
        let r = img("  ![[dossier/Photo.JPG|300]]  ").unwrap();
        assert_eq!(r.target, "dossier/Photo.JPG");
        assert_eq!(r.width, Some(300.0));
        let r = img("![[a.webp|alt|120x80]]").unwrap();
        assert_eq!((r.width, r.height), (Some(120.0), Some(80.0)));
        assert_eq!(img("![[note]]"), None);
        assert_eq!(img("![[Note.md]]"), None);
        assert_eq!(img("texte ![[a.png]]"), None);
        assert_eq!(img("![[a.png]] suite"), None);
        assert_eq!(img("![[a.png]] ![[b.png]]"), None);
        assert_eq!(img("![[a.pdf]]"), None);
    }

    #[test]
    fn markdown_images() {
        let r = img("![](img/mon%20image.png)").unwrap();
        assert_eq!(r.target, "img/mon image.png");
        let r = img("![légende|250](a.jpeg)").unwrap();
        assert_eq!(r.width, Some(250.0));
        let r = img("![x](<dossier avec espace/b.gif>)").unwrap();
        assert_eq!(r.target, "dossier avec espace/b.gif");
        let r = img("![x](a.png \"titre\")").unwrap();
        assert_eq!(r.target, "a.png");
        assert_eq!(img("![x](https://x.y/a.png)"), None);
        assert_eq!(img("![x](a.txt)"), None);
        assert_eq!(img("[x](a.png)"), None);
        assert_eq!(img("avant ![x](a.png)"), None);
    }

    #[test]
    fn sizes() {
        let r = ImageRef {
            target: "a.png".into(),
            width: Some(300.0),
            height: None,
        };
        assert_eq!(display_size((600, 400), &r, 800.0), (300.0, 200.0));
        // Never wider than the column.
        let (w, h) = display_size((600, 400), &r, 150.0);
        assert!((w - 150.0).abs() < 0.01 && (h - 100.0).abs() < 0.01);
        let r = ImageRef {
            target: "a.png".into(),
            width: None,
            height: None,
        };
        assert_eq!(display_size((100, 50), &r, 800.0), (100.0, 50.0));
        // Very tall images are capped.
        let (w, h) = display_size((100, 3000), &r, 800.0);
        assert_eq!(h, MAX_HEIGHT);
        assert!((w - 20.0).abs() < 0.01);
    }

    #[test]
    fn decoding() {
        assert_eq!(percent_decode("a%20b%C3%A9"), "a bé");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}
