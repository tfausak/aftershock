extern crate encoding;

use encoding::all::WINDOWS_1252;
use encoding::all::UTF_16LE;
use encoding::Encoding;
use std::collections::HashMap;

// Types //////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct Replay {
    pub header: Section<Header>,
    pub content: Section<Content>,
}

#[derive(Debug)]
pub struct Section<T> {
    pub size: u32,
    pub crc: u32,
    pub value: T,
}

#[derive(Debug)]
pub struct Header {
    pub version: Version,
    pub label: String,
    pub properties: Dictionary<Property>,
}

#[derive(Debug)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: Option<u32>,
}

pub type Dictionary<T> = HashMap<String, T>;

#[derive(Debug)]
pub struct Property {
    pub label: String,
    pub size: u64,
    pub value: PropertyValue,
}

#[derive(Debug)]
pub enum PropertyValue {
    Array(Vec<Dictionary<Property>>),
    Bool(u8),
    Byte(String, Option<String>),
    Float(f32),
    Int(i32),
    Name(String),
    QWord(u64),
    Str(String),
}

#[derive(Debug)]
pub struct Content {
    pub levels: Vec<String>,
    pub keyframes: Vec<Keyframe>,
    pub frames: Vec<Frame>,
    pub messages: Vec<Message>,
    pub marks: Vec<Mark>,
    pub packages: Vec<String>,
    pub objects: Vec<String>,
    pub names: Vec<String>,
    pub classes: Vec<ClassIndex>,
    pub hierarchy: Vec<ClassInfo>,
}

#[derive(Debug)]
pub struct Keyframe {
    pub time: f32,
    pub frame: u32,
    pub position: u32,
}

#[derive(Debug)]
pub struct Frame {
  // TODO: Add frame fields.
}

#[derive(Debug)]
pub struct Message {
    pub frame: u32,
    pub label: String,
    pub value: String,
}

#[derive(Debug)]
pub struct Mark {
    pub value: String,
    pub frame: u32,
}

#[derive(Debug)]
pub struct ClassIndex {
    pub name: String,
    pub index: u32,
}

#[derive(Debug)]
pub struct ClassInfo {
    pub index: u32,
    pub parent: u32,
    pub id: u32,
    pub attributes: Vec<AttributeIndex>,
}

#[derive(Debug)]
pub struct AttributeIndex {
    pub index: u32,
    pub id: u32,
}

pub type Get<T> = fn(&[u8], usize) -> GetResult<T>;

pub type GetResult<T> = Result<(usize, T), GetError>;

#[derive(Debug)]
pub enum GetError {
    IndexOutOfBounds { len: usize, index: usize },
    InvalidUtf16(String),
    InvalidWindows1252(String),
    NotImplemented,
    UnknownProperty(String),
}

// Parsing ////////////////////////////////////////////////////////////////////

pub fn get_replay(bytes: &[u8], index: usize) -> GetResult<Replay> {
    let (index, header) = get_section(bytes, index, get_header)?;
    let (index, content) = get_section(bytes, index, get_content)?;
    Ok((index, Replay { header, content }))
}

pub fn get_section<T>(bytes: &[u8], index: usize, get_value: Get<T>) -> GetResult<Section<T>> {
    let (index, size) = get_u32(bytes, index)?;
    let (index, crc) = get_u32(bytes, index)?;
    let (index, value) = get_value(bytes, index)?;
    Ok((index, Section { size, crc, value }))
}

pub fn get_header(bytes: &[u8], index: usize) -> GetResult<Header> {
    let (index, version) = get_version(bytes, index)?;
    let (index, label) = get_string(bytes, index)?;
    let (index, properties) = get_dictionary(bytes, index, get_property)?;
    Ok((
        index,
        Header {
            version,
            label,
            properties,
        },
    ))
}

pub fn get_version(bytes: &[u8], index: usize) -> GetResult<Version> {
    let (index, major) = get_u32(bytes, index)?;
    let (index, minor) = get_u32(bytes, index)?;
    let (index, patch) = get_option(bytes, index, major >= 868 && minor >= 18, get_u32)?;
    Ok((
        index,
        Version {
            major,
            minor,
            patch,
        },
    ))
}

pub fn get_property(bytes: &[u8], index: usize) -> GetResult<Property> {
    let (index, label) = get_string(bytes, index)?;
    let (index, size) = get_u64(bytes, index)?;
    let (index, value) = get_property_value(bytes, index, &label)?;
    Ok((index, Property { label, size, value }))
}

pub fn get_property_value(bytes: &[u8], index: usize, label: &String) -> GetResult<PropertyValue> {
    match label.as_ref() {
        "ArrayProperty" => {
            let (index, x) = get_vec(bytes, index, |b, i| get_dictionary(b, i, get_property))?;
            Ok((index, PropertyValue::Array(x)))
        }
        "BoolProperty" => {
            let (index, x) = get_u8(bytes, index)?;
            Ok((index, PropertyValue::Bool(x)))
        }
        "ByteProperty" => {
            let (index, k) = get_string(bytes, index)?;
            let (index, v) = get_option(bytes, index, k != "OnlinePlatform_Steam", get_string)?;
            Ok((index, PropertyValue::Byte(k, v)))
        }
        "FloatProperty" => {
            let (index, x) = get_f32(bytes, index)?;
            Ok((index, PropertyValue::Float(x)))
        }
        "IntProperty" => {
            let (index, x) = get_i32(bytes, index)?;
            Ok((index, PropertyValue::Int(x)))
        }
        "NameProperty" => {
            let (index, x) = get_string(bytes, index)?;
            Ok((index, PropertyValue::Name(x)))
        }
        "QWordProperty" => {
            let (index, x) = get_u64(bytes, index)?;
            Ok((index, PropertyValue::QWord(x)))
        }
        "StrProperty" => {
            let (index, x) = get_string(bytes, index)?;
            Ok((index, PropertyValue::Str(x)))
        }
        _ => Err(GetError::UnknownProperty(label.clone())),
    }
}

pub fn get_content(bytes: &[u8], index: usize) -> GetResult<Content> {
    let (index, levels) = get_vec(bytes, index, get_string)?;
    let (index, keyframes) = get_vec(bytes, index, get_keyframe)?;
    let (index, frames) = get_frames(bytes, index)?;
    let (index, messages) = get_vec(bytes, index, get_message)?;
    let (index, marks) = get_vec(bytes, index, get_mark)?;
    let (index, packages) = get_vec(bytes, index, get_string)?;
    let (index, objects) = get_vec(bytes, index, get_string)?;
    let (index, names) = get_vec(bytes, index, get_string)?;
    let (index, classes) = get_vec(bytes, index, get_class_index)?;
    let (index, hierarchy) = get_vec(bytes, index, get_class_info)?;
    Ok((
        index,
        Content {
            levels,
            keyframes,
            frames,
            messages,
            marks,
            packages,
            objects,
            names,
            classes,
            hierarchy,
        },
    ))
}

pub fn get_keyframe(bytes: &[u8], index: usize) -> GetResult<Keyframe> {
    let (index, time) = get_f32(bytes, index)?;
    let (index, frame) = get_u32(bytes, index)?;
    let (index, position) = get_u32(bytes, index)?;
    Ok((
        index,
        Keyframe {
            time,
            frame,
            position,
        },
    ))
}

pub fn get_frames(bytes: &[u8], index: usize) -> GetResult<Vec<Frame>> {
    let (index, size) = get_u32(bytes, index)?;
    let (index, _bytes) = get_raw(bytes, index, size as usize)?;
    // TODO: Parse frames.
    Ok((index, Vec::new()))
}

pub fn get_frame(_bytes: &[u8], _index: usize) -> GetResult<Frame> {
    Err(GetError::NotImplemented)
}

pub fn get_message(bytes: &[u8], index: usize) -> GetResult<Message> {
    let (index, frame) = get_u32(bytes, index)?;
    let (index, label) = get_string(bytes, index)?;
    let (index, value) = get_string(bytes, index)?;
    Ok((
        index,
        Message {
            frame,
            label,
            value,
        },
    ))
}

pub fn get_mark(bytes: &[u8], index: usize) -> GetResult<Mark> {
    let (index, value) = get_string(bytes, index)?;
    let (index, frame) = get_u32(bytes, index)?;
    Ok((index, Mark { value, frame }))
}

pub fn get_class_index(bytes: &[u8], index: usize) -> GetResult<ClassIndex> {
    let (index, name) = get_string(bytes, index)?;
    let (index, ix) = get_u32(bytes, index)?;
    Ok((index, ClassIndex { name, index: ix }))
}

pub fn get_class_info(bytes: &[u8], index: usize) -> GetResult<ClassInfo> {
    let (index, ix) = get_u32(bytes, index)?;
    let (index, parent) = get_u32(bytes, index)?;
    let (index, id) = get_u32(bytes, index)?;
    let (index, attributes) = get_vec(bytes, index, get_attribute_index)?;
    Ok((
        index,
        ClassInfo {
            index: ix,
            parent,
            id,
            attributes,
        },
    ))
}

pub fn get_attribute_index(bytes: &[u8], index: usize) -> GetResult<AttributeIndex> {
    let (index, ix) = get_u32(bytes, index)?;
    let (index, id) = get_u32(bytes, index)?;
    Ok((index, AttributeIndex { index: ix, id }))
}

// Helpers ////////////////////////////////////////////////////////////////////

fn get_raw(bytes: &[u8], index: usize, size: usize) -> GetResult<&[u8]> {
    let end = index + size;
    match bytes.get(index..end) {
        Some(raw) => Ok((end, raw)),
        None => Err(GetError::IndexOutOfBounds {
            len: bytes.len(),
            index: end,
        }),
    }
}

fn get_f32(bytes: &[u8], index: usize) -> GetResult<f32> {
    let (index, x) = get_u32(bytes, index)?;
    Ok((index, x as f32))
}

fn get_i32(bytes: &[u8], index: usize) -> GetResult<i32> {
    let (index, x) = get_u32(bytes, index)?;
    Ok((index, x as i32))
}

fn get_u8(bytes: &[u8], index: usize) -> GetResult<u8> {
    let (index, raw) = get_raw(bytes, index, 1)?;
    Ok((index, raw[0]))
}

fn get_u32(bytes: &[u8], index: usize) -> GetResult<u32> {
    let (index, raw) = get_raw(bytes, index, 4)?;
    Ok((
        index,
        raw[0] as u32 | (raw[1] as u32) << 8 | (raw[2] as u32) << 16 | (raw[3] as u32) << 24,
    ))
}

fn get_u64(bytes: &[u8], index: usize) -> GetResult<u64> {
    let (index, raw) = get_raw(bytes, index, 8)?;
    Ok((
        index,
        raw[0] as u64 | (raw[1] as u64) << 8 | (raw[2] as u64) << 16 | (raw[3] as u64) << 24
            | (raw[4] as u64) << 32 | (raw[5] as u64) << 40 | (raw[6] as u64) << 48
            | (raw[7] as u64) << 56,
    ))
}

fn get_option<T>(
    bytes: &[u8],
    index: usize,
    condition: bool,
    get_value: Get<T>,
) -> GetResult<Option<T>> {
    if condition {
        let (index, value) = get_value(bytes, index)?;
        Ok((index, Some(value)))
    } else {
        Ok((index, None))
    }
}

fn get_string(bytes: &[u8], index: usize) -> GetResult<String> {
    let (index, size) = get_i32(bytes, index)?;
    if size < 0 {
        let (index, raw) = get_raw(bytes, index, ((-size - 1) as usize) * 2)?;
        match UTF_16LE.decode(raw, encoding::DecoderTrap::Strict) {
            Ok(string) => Ok((index + 2, String::from(string))),
            Err(error) => Err(GetError::InvalidUtf16(error.into_owned())),
        }
    } else {
        let size = if size == 0x0500_0000 { 8 } else { size };
        let (index, raw) = get_raw(bytes, index, size as usize - 1)?;
        match WINDOWS_1252.decode(raw, encoding::DecoderTrap::Strict) {
            Ok(string) => Ok((index + 1, String::from(string))),
            Err(error) => Err(GetError::InvalidWindows1252(error.into_owned())),
        }
    }
}

fn get_dictionary<T>(bytes: &[u8], index: usize, get_value: Get<T>) -> GetResult<Dictionary<T>>
where
    T: std::fmt::Debug,
{
    let mut dictionary = HashMap::new();
    let mut i = index;
    loop {
        let (j, key) = get_string(bytes, i)?;
        i = j;
        match key.as_ref() {
            "None" => break,
            "\x00\x00\x00None" => break,
            _ => {
                let (j, value) = get_value(bytes, i)?;
                i = j;
                dictionary.insert(key, value);
            }
        }
    }
    Ok((i, dictionary))
}

fn get_vec<T>(bytes: &[u8], index: usize, get_value: Get<T>) -> GetResult<Vec<T>>
where
    T: std::fmt::Debug,
{
    let (index, size) = get_u32(bytes, index)?;
    let mut vec = Vec::with_capacity(size as usize);
    let mut i = index;
    for _ in 0..size {
        let (j, value) = get_value(bytes, i)?;
        i = j;
        vec.push(value)
    }
    Ok((i, vec))
}
