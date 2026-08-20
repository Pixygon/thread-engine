//! In-world web reader — the **old web, inside the Thread**.
//!
//! Signboards and links on the Thread point at ordinary `https://` pages. Rather
//! than kick you out to an OS browser, this fetches the page on a background
//! thread, renders a **reader view** (title + readable text + followable links)
//! in an in-world panel, and lets you walk the 2D web without ever leaving the
//! Thread browser. It's a reader, not a full engine — no JS/CSS layout — but it
//! keeps you in-world and is dependency-free, robust, and testable. (A full
//! embedded page surface via an offscreen browser engine is the later upgrade.)

use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// A reader-view rendering of a page.
#[derive(Debug, Clone, Default)]
pub struct ReaderDoc {
    pub title: String,
    pub text: String,
    /// `(display text, absolute url)` for each followable link.
    pub links: Vec<(String, String)>,
}

enum State {
    Idle,
    Loading(String),
    Loaded(Box<ReaderDoc>),
    Error(String, String),
}

/// The in-world web reader (one page at a time, with back-history).
pub struct WebReader {
    state: State,
    url: Option<String>,
    history: Vec<String>,
    pending: Option<(String, Receiver<Result<ReaderDoc, String>>)>,
}

impl Default for WebReader {
    fn default() -> Self {
        Self::new()
    }
}

impl WebReader {
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            url: None,
            history: Vec::new(),
            pending: None,
        }
    }

    pub fn is_open(&self) -> bool {
        !matches!(self.state, State::Idle)
    }
    pub fn can_go_back(&self) -> bool {
        self.history.len() > 1
    }

    pub fn close(&mut self) {
        self.state = State::Idle;
        self.url = None;
        self.history.clear();
        self.pending = None;
    }

    /// Open a URL (pushed onto history).
    pub fn open(&mut self, url: &str) {
        self.history.push(url.to_string());
        self.load(url.to_string());
    }

    /// Go back to the previous page.
    pub fn back(&mut self) {
        if self.history.len() > 1 {
            self.history.pop();
            if let Some(prev) = self.history.last().cloned() {
                self.load(prev);
            }
        }
    }

    fn load(&mut self, url: String) {
        let (tx, rx) = channel();
        let fetch_url = url.clone();
        std::thread::spawn(move || {
            let _ = tx.send(fetch_reader(&fetch_url));
        });
        self.state = State::Loading(url.clone());
        self.url = Some(url.clone());
        self.pending = Some((url, rx));
    }

    /// Collect a finished fetch, if any. Call once per frame.
    pub fn poll(&mut self) {
        let Some((url, rx)) = &self.pending else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(doc)) => {
                self.state = State::Loaded(Box::new(doc));
                self.pending = None;
            }
            Ok(Err(e)) => {
                self.state = State::Error(url.clone(), e);
                self.pending = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                let u = url.clone();
                self.state = State::Error(u, "fetch stopped".into());
                self.pending = None;
            }
        }
    }

    /// What to draw this frame (owned, so egui closures stay free).
    pub fn view(&self) -> Option<WebView> {
        match &self.state {
            State::Idle => None,
            State::Loading(url) => Some(WebView {
                url: url.clone(),
                title: String::new(),
                text: String::new(),
                links: Vec::new(),
                loading: true,
                error: None,
            }),
            State::Loaded(d) => Some(WebView {
                url: self.url.clone().unwrap_or_default(),
                title: if d.title.is_empty() {
                    self.url.clone().unwrap_or_default()
                } else {
                    d.title.clone()
                },
                text: d.text.clone(),
                links: d.links.clone(),
                loading: false,
                error: None,
            }),
            State::Error(url, msg) => Some(WebView {
                url: url.clone(),
                title: url.clone(),
                text: String::new(),
                links: Vec::new(),
                loading: false,
                error: Some(msg.clone()),
            }),
        }
    }
}

/// An owned snapshot for rendering.
#[derive(Clone)]
pub struct WebView {
    pub url: String,
    pub title: String,
    pub text: String,
    pub links: Vec<(String, String)>,
    pub loading: bool,
    pub error: Option<String>,
}

/// Fetch a URL and render it to a [`ReaderDoc`] (blocking; run on a bg thread).
fn fetch_reader(url: &str) -> Result<ReaderDoc, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let resp = reqwest::get(url).await.map_err(|e| e.to_string())?;
        let status = resp.status();
        let final_url = resp.url().to_string();
        let body = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("HTTP {}", status.as_u16()));
        }
        Ok(read_html(&body, &final_url))
    })
}

// ---------------------------------------------------------------------------
// Reader-view extraction (dependency-free HTML → title + text + links).
// ---------------------------------------------------------------------------

/// Turn raw HTML into a reader view: title, readable text, absolute links.
pub fn read_html(html: &str, base_url: &str) -> ReaderDoc {
    let cleaned = remove_blocks(html);
    let title = extract_tag_text(&cleaned, "title").unwrap_or_default();
    let links = extract_links(&cleaned, base_url);
    let text = to_text(&cleaned);
    ReaderDoc {
        title: decode_entities(&title).trim().to_string(),
        text,
        links,
    }
}

/// Strip `<script>`, `<style>` and comments (with their contents).
fn remove_blocks(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &lower[i..];
        if let Some(end) = block_end(rest, "<script", "</script>")
            .or_else(|| block_end(rest, "<style", "</style>"))
            .or_else(|| comment_end(rest))
        {
            i += end;
        } else {
            out.push(html[i..].chars().next().unwrap_or(' '));
            i += html[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        }
    }
    out
}

fn block_end(rest: &str, open: &str, close: &str) -> Option<usize> {
    if rest.starts_with(open) {
        return Some(
            rest.find(close)
                .map(|e| e + close.len())
                .unwrap_or(rest.len()),
        );
    }
    None
}

fn comment_end(rest: &str) -> Option<usize> {
    if rest.starts_with("<!--") {
        return Some(rest.find("-->").map(|e| e + 3).unwrap_or(rest.len()));
    }
    None
}

/// Text inside the first `<tag>…</tag>` (case-insensitive).
fn extract_tag_text(html: &str, tag: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = format!("<{tag}");
    let start = lower.find(&open)?;
    let gt = lower[start..].find('>')? + start + 1;
    let end = lower[gt..].find(&format!("</{tag}"))? + gt;
    Some(html[gt..end].to_string())
}

/// Pull `<a href>` links, resolving each to an absolute URL against `base`.
fn extract_links(html: &str, base: &str) -> Vec<(String, String)> {
    let base_url = reqwest::Url::parse(base).ok();
    let lower = html.to_ascii_lowercase();
    let mut links = Vec::new();
    let mut search = 0;
    while let Some(rel) = lower[search..].find("<a ") {
        let at = search + rel;
        let Some(gt) = lower[at..].find('>') else {
            break;
        };
        let tag_end = at + gt;
        let tag = &html[at..tag_end];
        let href = attr_value(tag, "href");
        // Inner text up to </a>.
        let inner_end = lower[tag_end..]
            .find("</a")
            .map(|e| tag_end + e)
            .unwrap_or(tag_end);
        let text = decode_entities(&to_text(&html[tag_end + 1..inner_end]))
            .trim()
            .to_string();
        search = inner_end + 3;
        if let Some(href) = href {
            if href.starts_with('#')
                || href.starts_with("javascript:")
                || href.starts_with("mailto:")
            {
                continue;
            }
            let abs = base_url
                .as_ref()
                .and_then(|b| b.join(&href).ok())
                .map(|u| u.to_string())
                .unwrap_or(href);
            if abs.starts_with("http") && !text.is_empty() {
                links.push((text, abs));
            }
        }
    }
    links.truncate(80);
    links
}

/// Read a quoted attribute value out of a start tag.
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let key = format!("{name}=");
    let idx = lower.find(&key)? + key.len();
    let rest = &tag[idx..];
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let end = rest[1..].find(quote)? + 1;
        Some(rest[1..end].to_string())
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(rest.len());
        Some(rest[..end].to_string())
    }
}

/// Strip tags to readable text: block tags become line breaks, whitespace
/// collapses, entities decode.
fn to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let end = html[i..]
                .find('>')
                .map(|e| i + e + 1)
                .unwrap_or(bytes.len());
            let tag = html[i..end].to_ascii_lowercase();
            if is_break_tag(&tag) {
                out.push('\n');
            } else {
                out.push(' ');
            }
            i = end;
        } else {
            out.push(html[i..].chars().next().unwrap_or(' '));
            i += html[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        }
    }
    collapse_ws(&decode_entities(&out))
}

fn is_break_tag(tag: &str) -> bool {
    for t in [
        "</p",
        "<br",
        "</div",
        "</h1",
        "</h2",
        "</h3",
        "</h4",
        "</li",
        "</tr",
        "</section",
        "</article",
        "</header",
        "</footer",
    ] {
        if tag.starts_with(t) {
            return true;
        }
    }
    false
}

/// Collapse runs of spaces, and runs of blank lines to at most one.
fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank_run = 0;
    for line in s.lines() {
        let mut trimmed = String::with_capacity(line.len());
        let mut prev_space = false;
        for c in line.chars() {
            if c.is_whitespace() {
                if !prev_space {
                    trimmed.push(' ');
                }
                prev_space = true;
            } else {
                trimmed.push(c);
                prev_space = false;
            }
        }
        let t = trimmed.trim();
        if t.is_empty() {
            blank_run += 1;
            if blank_run <= 1 && !out.is_empty() {
                out.push('\n');
            }
        } else {
            blank_run = 0;
            out.push_str(t);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

/// Decode the common HTML entities (named + numeric).
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let rest = &s[i..];
        let Some(semi) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            continue;
        };
        let ent = &rest[1..semi];
        let decoded = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            "mdash" | "#8212" => Some('—'),
            "ndash" | "#8211" => Some('–'),
            "hellip" | "#8230" => Some('…'),
            "rsquo" | "#8217" => Some('\u{2019}'),
            "lsquo" | "#8216" => Some('\u{2018}'),
            "ldquo" | "#8220" => Some('\u{201C}'),
            "rdquo" | "#8221" => Some('\u{201D}'),
            _ => ent
                .strip_prefix('#')
                .and_then(|n| n.parse::<u32>().ok())
                .and_then(char::from_u32),
        };
        if let Some(d) = decoded {
            out.push(d);
            for _ in 0..(semi) {
                chars.next();
            }
        } else {
            out.push('&');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_title_text_and_absolute_links() {
        let html = r##"
          <html><head><title>Pixiel &amp; Co</title>
          <style>body{color:red}</style></head>
          <body>
            <script>var x = '<not text>';</script>
            <h1>The Forge</h1>
            <p>Where prompts are hammered into worlds.</p>
            <a href="/about">About us</a>
            <a href="https://pixygon.io/thread">The Thread</a>
            <a href="#top">skip</a>
          </body></html>
        "##;
        let doc = read_html(html, "https://pixiel.ai/forge");
        assert_eq!(doc.title, "Pixiel & Co");
        assert!(doc.text.contains("The Forge"));
        assert!(doc.text.contains("hammered into worlds"));
        assert!(!doc.text.contains("var x"), "script contents stripped");
        assert!(!doc.text.contains("color:red"), "style contents stripped");
        // Relative link resolved to absolute; fragment link dropped.
        assert!(doc
            .links
            .iter()
            .any(|(t, u)| t == "About us" && u == "https://pixiel.ai/about"));
        assert!(doc
            .links
            .iter()
            .any(|(_, u)| u == "https://pixygon.io/thread"));
        assert!(!doc.links.iter().any(|(t, _)| t == "skip"));
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(
            decode_entities("a &amp; b &lt;c&gt; &#39;d&#39; &mdash; e"),
            "a & b <c> 'd' — e"
        );
    }
}
