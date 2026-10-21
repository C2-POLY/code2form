//! A small XML reader, enough for SVG, AMF and 3MF: elements with their
//! attributes and text. Comments, processing instructions and the DOCTYPE
//! are skipped; namespace prefixes are dropped from names.

#[derive(Debug, Default)]
pub(super) struct Element {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<Element>,
    pub text: String,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// The child elements called `name`.
    pub fn named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Every element below this one called `name`, in document order.
    pub fn descendants<'a>(&'a self, name: &str) -> Vec<&'a Element> {
        let mut out = Vec::new();
        let mut stack = vec![self];
        while let Some(e) = stack.pop() {
            for child in e.children.iter().rev() {
                stack.push(child);
            }
            if e.name == name && !std::ptr::eq(e, self) {
                out.push(e);
            }
        }
        out
    }
}

/// Frees deep documents without recursing.
impl Drop for Element {
    fn drop(&mut self) {
        let mut pending = std::mem::take(&mut self.children);
        while let Some(mut element) = pending.pop() {
            pending.append(&mut element.children);
        }
    }
}

/// The root element of `text`.
pub(super) fn parse(text: &str) -> Result<Element, String> {
    let bytes = text.as_bytes();
    let mut stack: Vec<Element> = vec![Element::default()];
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            let end = text[i..].find('<').map_or(text.len(), |n| i + n);
            let top = stack.last_mut().expect("the document element is never popped");
            top.text.push_str(&decode(&text[i..end]));
            i = end;
            continue;
        }
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->").ok_or("unterminated comment")? + 3;
        } else if let Some(cdata) = rest.strip_prefix("<![CDATA[") {
            let end = cdata.find("]]>").ok_or("unterminated CDATA")?;
            stack.last_mut().expect("document").text.push_str(&cdata[..end]);
            i += 9 + end + 3;
        } else if rest.starts_with("<?") {
            i += rest.find("?>").ok_or("unterminated processing instruction")? + 2;
        } else if rest.starts_with("<!") {
            // DOCTYPE, possibly with an internal subset in brackets.
            let mut depth = 0;
            let mut j = 2;
            while j < rest.len() {
                match rest.as_bytes()[j] {
                    b'[' => depth += 1,
                    b']' => depth -= 1,
                    b'>' if depth <= 0 => break,
                    _ => {}
                }
                j += 1;
            }
            i += j + 1;
        } else if let Some(close) = rest.strip_prefix("</") {
            let end = close.find('>').ok_or("unterminated end tag")?;
            let name = local(close[..end].trim());
            if stack.len() < 2 {
                return Err(format!("unexpected </{name}>"));
            }
            let element = stack.pop().expect("checked above");
            if element.name != name {
                return Err(format!("</{name}> closes <{}>", element.name));
            }
            stack.last_mut().expect("checked above").children.push(element);
            i += 2 + end + 1;
        } else {
            let (element, length, empty) = start_tag(&rest[1..])?;
            i += 1 + length;
            if empty {
                stack.last_mut().expect("document").children.push(element);
            } else {
                stack.push(element);
            }
        }
    }
    if stack.len() != 1 {
        return Err(format!("<{}> is not closed", stack.last().map_or("", |e| e.name.as_str())));
    }
    let mut document = stack.pop().expect("one element left");
    std::mem::take(&mut document.children).into_iter().next().ok_or_else(|| "no root element".to_string())
}

/// A start tag after its `<`: the element, the bytes read and whether it
/// closes itself (`/>`).
fn start_tag(text: &str) -> Result<(Element, usize, bool), String> {
    let bytes = text.as_bytes();
    let name_end = text.find(|c: char| c.is_whitespace() || c == '>' || c == '/').ok_or("unterminated tag")?;
    let mut element =
        Element { name: local(&text[..name_end]).to_string(), attributes: Vec::new(), children: Vec::new(), text: String::new() };
    let mut i = name_end;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        match bytes.get(i) {
            None => return Err("unterminated tag".into()),
            Some(b'>') => return Ok((element, i + 1, false)),
            Some(b'/') if bytes.get(i + 1) == Some(&b'>') => return Ok((element, i + 2, true)),
            _ => {}
        }
        let eq = text[i..].find('=').ok_or("attribute without value")? + i;
        let name = text[i..eq].trim();
        let mut j = eq + 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let quote = *bytes.get(j).ok_or("unterminated tag")?;
        if quote != b'"' && quote != b'\'' {
            return Err(format!("unquoted value for '{name}'"));
        }
        let end = text[j + 1..].find(quote as char).ok_or("unterminated attribute")? + j + 1;
        // Prefixed attributes (xlink:href) keep their prefix.
        element.attributes.push((name.to_string(), decode(&text[j + 1..end])));
        i = end + 1;
    }
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Replaces character and predefined entity references.
fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';') else { break };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_attributes_and_text() {
        let doc = parse(
            "<?xml version=\"1.0\"?>\n<!DOCTYPE svg [ <!ENTITY x \"y\"> ]>\n<!-- c -->\
             <svg:svg a='1' b=\"x &amp; y\"><g><rect x=\"2\"/></g><t>a&lt;b<![CDATA[<c>]]></t></svg:svg>",
        )
        .unwrap();
        assert_eq!(doc.name, "svg");
        assert_eq!(doc.attr("b"), Some("x & y"));
        assert_eq!(doc.descendants("rect")[0].attr("x"), Some("2"));
        assert_eq!(doc.child("t").unwrap().text, "a<b<c>");
    }

    #[test]
    fn mismatched_tags_fail() {
        assert!(parse("<a><b></a>").is_err());
        assert!(parse("<a>").is_err());
    }
}
