//! Minimal IPP/1.1–2.0 message codec (RFC 8010).

pub mod tag {
    pub const OPERATION: u8 = 0x01;
    pub const JOB: u8 = 0x02;
    pub const END: u8 = 0x03;
    pub const PRINTER: u8 = 0x04;
    pub const UNSUPPORTED: u8 = 0x05;

    pub const NO_VALUE: u8 = 0x13;
    pub const INTEGER: u8 = 0x21;
    pub const BOOLEAN: u8 = 0x22;
    pub const ENUM: u8 = 0x23;
    pub const OCTET: u8 = 0x30;
    pub const RESOLUTION: u8 = 0x32;
    pub const RANGE: u8 = 0x33;
    pub const BEG_COLLECTION: u8 = 0x34;
    pub const END_COLLECTION: u8 = 0x37;
    pub const TEXT: u8 = 0x41;
    pub const NAME: u8 = 0x42;
    pub const KEYWORD: u8 = 0x44;
    pub const URI: u8 = 0x45;
    pub const URI_SCHEME: u8 = 0x46;
    pub const CHARSET: u8 = 0x47;
    pub const LANGUAGE: u8 = 0x48;
    pub const MIME: u8 = 0x49;
    pub const MEMBER_NAME: u8 = 0x4A;
}

pub mod op {
    pub const PRINT_JOB: u16 = 0x0002;
    pub const VALIDATE_JOB: u16 = 0x0004;
    pub const CREATE_JOB: u16 = 0x0005;
    pub const SEND_DOCUMENT: u16 = 0x0006;
    pub const CANCEL_JOB: u16 = 0x0008;
    pub const GET_JOB_ATTRIBUTES: u16 = 0x0009;
    pub const GET_JOBS: u16 = 0x000A;
    pub const GET_PRINTER_ATTRIBUTES: u16 = 0x000B;
    pub const CLOSE_JOB: u16 = 0x003B;
    pub const IDENTIFY_PRINTER: u16 = 0x003C;
}

pub mod status {
    pub const OK: u16 = 0x0000;
    pub const BAD_REQUEST: u16 = 0x0400;
    pub const NOT_FOUND: u16 = 0x0406;
    pub const DOCUMENT_FORMAT_NOT_SUPPORTED: u16 = 0x040A;
    pub const OPERATION_NOT_SUPPORTED: u16 = 0x0501;
    pub const DEVICE_ERROR: u16 = 0x0504;
    pub const NOT_ACCEPTING: u16 = 0x0506;
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i32),
    Bool(bool),
    Enum(i32),
    Range(i32, i32),
    Resolution(i32, i32),
    /// String-like values with their tag (keyword, uri, text…).
    Str(u8, String),
    Octets(Vec<u8>),
    Collection(Vec<(String, Vec<Value>)>),
    NoValue,
}

impl Value {
    pub fn kw(s: &str) -> Value {
        Value::Str(tag::KEYWORD, s.into())
    }
    pub fn text(s: &str) -> Value {
        Value::Str(tag::TEXT, s.into())
    }
    pub fn name(s: &str) -> Value {
        Value::Str(tag::NAME, s.into())
    }
    pub fn uri(s: &str) -> Value {
        Value::Str(tag::URI, s.into())
    }
    pub fn mime(s: &str) -> Value {
        Value::Str(tag::MIME, s.into())
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Value::Str(_, s) = self {
            Some(s)
        } else {
            None
        }
    }
    pub fn as_int(&self) -> Option<i32> {
        match self {
            Value::Int(i) | Value::Enum(i) => Some(*i),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Group {
    pub tag: u8,
    pub attrs: Vec<(String, Vec<Value>)>,
}

impl Group {
    pub fn new(tag: u8) -> Group {
        Group { tag, attrs: Vec::new() }
    }
    pub fn add(&mut self, name: &str, v: Value) -> &mut Self {
        self.attrs.push((name.into(), vec![v]));
        self
    }
    pub fn add_many(&mut self, name: &str, v: Vec<Value>) -> &mut Self {
        self.attrs.push((name.into(), v));
        self
    }
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.attrs.iter().find(|(n, _)| n == name).and_then(|(_, v)| v.first())
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub version: (u8, u8),
    /// Operation id in requests, status code in responses.
    pub code: u16,
    pub request_id: u32,
    pub groups: Vec<Group>,
    pub data: Vec<u8>,
}

impl Message {
    pub fn group(&self, tag: u8) -> Option<&Group> {
        self.groups.iter().find(|g| g.tag == tag)
    }
    pub fn op_attr(&self, name: &str) -> Option<&Value> {
        self.group(tag::OPERATION).and_then(|g| g.get(name))
    }
    pub fn job_attr(&self, name: &str) -> Option<&Value> {
        self.group(tag::JOB).and_then(|g| g.get(name))
    }

    pub fn response(req: &Message, status: u16) -> Message {
        let mut op = Group::new(tag::OPERATION);
        op.add("attributes-charset", Value::Str(tag::CHARSET, "utf-8".into()));
        op.add("attributes-natural-language", Value::Str(tag::LANGUAGE, "en".into()));
        Message { version: (2, 0).min(req.version), code: status, request_id: req.request_id, groups: vec![op], data: Vec::new() }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.i..self.i + n)?;
        self.i += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|s| s[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|s| u16::from_be_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
}

fn decode_value(t: u8, raw: &[u8]) -> Value {
    let i32_at = |o: usize| raw.get(o..o + 4).map(|s| i32::from_be_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0);
    match t {
        tag::INTEGER => Value::Int(i32_at(0)),
        tag::ENUM => Value::Enum(i32_at(0)),
        tag::BOOLEAN => Value::Bool(raw.first() == Some(&1)),
        tag::RANGE => Value::Range(i32_at(0), i32_at(4)),
        tag::RESOLUTION => Value::Resolution(i32_at(0), i32_at(4)),
        0x10..=0x1F => Value::NoValue,
        0x40..=0x5F => Value::Str(t, String::from_utf8_lossy(raw).into_owned()),
        _ => Value::Octets(raw.to_vec()),
    }
}

pub fn parse(b: &[u8]) -> Option<Message> {
    let mut r = Reader { b, i: 0 };
    let version = (r.u8()?, r.u8()?);
    let code = r.u16()?;
    let request_id = r.u32()?;
    let mut groups: Vec<Group> = Vec::new();
    // Stack for nested collections: (member name being filled, members).
    let mut stack: Vec<(String, Vec<(String, Vec<Value>)>)> = Vec::new();
    let mut member: Option<String> = None;
    loop {
        let t = r.u8()?;
        if t == tag::END {
            break;
        }
        if t < 0x10 {
            groups.push(Group::new(t));
            continue;
        }
        let nlen = r.u16()? as usize;
        let name = String::from_utf8_lossy(r.take(nlen)?).into_owned();
        let vlen = r.u16()? as usize;
        let raw = r.take(vlen)?;
        if !stack.is_empty() {
            match t {
                tag::MEMBER_NAME => member = Some(String::from_utf8_lossy(raw).into_owned()),
                tag::BEG_COLLECTION => {
                    stack.push((member.take().unwrap_or_default(), Vec::new()));
                }
                tag::END_COLLECTION => {
                    let (mname, members) = stack.pop()?;
                    let v = Value::Collection(members);
                    if let Some(parent) = stack.last_mut() {
                        parent.1.push((mname, vec![v]));
                    } else if let Some(g) = groups.last_mut() {
                        push_value(g, &mname, v);
                    }
                }
                _ => {
                    let v = decode_value(t, raw);
                    let top = stack.last_mut()?;
                    match &member {
                        Some(m) => top.1.push((m.clone(), vec![v])),
                        // Additional value of the previous member.
                        None => {
                            if let Some(last) = top.1.last_mut() {
                                last.1.push(v);
                            }
                        }
                    }
                    member = None;
                }
            }
            continue;
        }
        if t == tag::BEG_COLLECTION {
            stack.push((name.clone(), Vec::new()));
            // Name kept as the attribute name when the collection closes.
            continue;
        }
        let g = groups.last_mut()?;
        push_value(g, &name, decode_value(t, raw));
    }
    Some(Message { version, code, request_id, groups, data: b[r.i..].to_vec() })
}

fn push_value(g: &mut Group, name: &str, v: Value) {
    if name.is_empty() {
        if let Some(last) = g.attrs.last_mut() {
            last.1.push(v);
        }
    } else {
        g.attrs.push((name.to_string(), vec![v]));
    }
}

fn put_str(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s);
}

fn encode_value(out: &mut Vec<u8>, name: &str, v: &Value) {
    let (t, raw): (u8, Vec<u8>) = match v {
        Value::Int(i) => (tag::INTEGER, i.to_be_bytes().to_vec()),
        Value::Enum(i) => (tag::ENUM, i.to_be_bytes().to_vec()),
        Value::Bool(b) => (tag::BOOLEAN, vec![*b as u8]),
        Value::Range(a, b) => (tag::RANGE, [a.to_be_bytes(), b.to_be_bytes()].concat()),
        Value::Resolution(x, y) => (tag::RESOLUTION, [x.to_be_bytes().as_slice(), &y.to_be_bytes(), &[3u8]].concat()),
        Value::Str(t, s) => (*t, s.as_bytes().to_vec()),
        Value::Octets(o) => (tag::OCTET, o.clone()),
        Value::NoValue => (tag::NO_VALUE, Vec::new()),
        Value::Collection(members) => {
            out.push(tag::BEG_COLLECTION);
            put_str(out, name.as_bytes());
            put_str(out, b"");
            for (m, vals) in members {
                for (i, mv) in vals.iter().enumerate() {
                    if i == 0 {
                        out.push(tag::MEMBER_NAME);
                        put_str(out, b"");
                        put_str(out, m.as_bytes());
                    }
                    encode_value(out, "", mv);
                }
            }
            out.push(tag::END_COLLECTION);
            put_str(out, b"");
            put_str(out, b"");
            return;
        }
    };
    out.push(t);
    put_str(out, name.as_bytes());
    put_str(out, &raw);
}

pub fn encode(m: &Message) -> Vec<u8> {
    let mut out = vec![m.version.0, m.version.1];
    out.extend_from_slice(&m.code.to_be_bytes());
    out.extend_from_slice(&m.request_id.to_be_bytes());
    for g in &m.groups {
        out.push(g.tag);
        for (name, vals) in &g.attrs {
            for (i, v) in vals.iter().enumerate() {
                encode_value(&mut out, if i == 0 { name } else { "" }, v);
            }
        }
    }
    out.push(tag::END);
    out.extend_from_slice(&m.data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_collection() {
        let mut g = Group::new(tag::OPERATION);
        g.add("attributes-charset", Value::Str(tag::CHARSET, "utf-8".into()));
        g.add_many("document-format-supported", vec![Value::mime("image/pwg-raster"), Value::mime("image/urf")]);
        g.add(
            "media-col",
            Value::Collection(vec![(
                "media-size".into(),
                vec![Value::Collection(vec![("x-dimension".into(), vec![Value::Int(5800)]), ("y-dimension".into(), vec![Value::Int(21000)])])],
            )]),
        );
        g.add("copies", Value::Int(2));
        let m = Message { version: (2, 0), code: 0x000B, request_id: 7, groups: vec![g], data: b"DATA".to_vec() };
        let back = parse(&encode(&m)).unwrap();
        assert_eq!(back.code, 0x000B);
        assert_eq!(back.request_id, 7);
        let g = back.group(tag::OPERATION).unwrap();
        assert_eq!(g.attrs.iter().find(|a| a.0 == "document-format-supported").unwrap().1.len(), 2);
        assert_eq!(g.get("copies"), Some(&Value::Int(2)));
        let Some(Value::Collection(mc)) = g.get("media-col") else { panic!("{:?}", g) };
        let Value::Collection(size) = &mc[0].1[0] else { panic!() };
        assert_eq!(size[1].1[0], Value::Int(21000));
        assert_eq!(back.data, b"DATA");
    }
}
