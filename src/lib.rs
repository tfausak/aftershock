use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;

type GetResult<T> = Result<T, GetError>;

#[derive(Debug)]
pub enum GetError {
    BitGet(BitGetError),
    ChecksumMismatch { expected: u32, actual: u32 },
    IndexOutOfBounds { index: usize, len: usize },
    InvalidUtf16(Vec<u8>),
    InvalidWindows1252(Vec<u8>),
    UnknownProperty(String),
}

pub struct Get {
    bytes: Vec<u8>,
    index: usize,
}

impl Get {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, index: 0 }
    }
}

type BitGetResult<T> = Result<T, BitGetError>;

#[derive(Debug)]
pub enum BitGetError {
    IndexOutOfBounds { index: usize, len: usize },
    UnknownActor(u32),
    UnknownAttribute(String),
    UnknownAttributeIndex(u32),
    UnknownClass(u32),
    UnknownName(u32),
    UnknownObject(u32),
    UnknownObjectClass(String),
    UnknownStreamId(u32),
}

struct BitGet {
    bytes: Vec<u8>,
    byte_index: usize,
    bit_index: usize,
}

impl BitGet {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            byte_index: 0,
            bit_index: 0,
        }
    }
}

#[derive(Debug)]
pub struct Replay {
    header: Section<Header>,
    content: Section<Content>,
}

impl Get {
    pub fn get_replay(&mut self) -> GetResult<Replay> {
        let header = self.get_section(Self::get_header)?;
        let content = self.get_section(|this| this.get_content(&header.value))?;
        Ok(Replay { header, content })
    }
}

#[derive(Debug)]
struct Section<T> {
    size: u32,
    crc: u32,
    value: T,
}

impl Get {
    fn get_section<F, T>(&mut self, get_value: F) -> GetResult<Section<T>>
    where
        F: Fn(&mut Self) -> GetResult<T>,
    {
        let size = self.get_u32()?;
        let crc = self.get_u32()?;
        self.check_crc_32(size, crc)?;
        let value = get_value(self)?;
        Ok(Section { size, crc, value })
    }

    fn check_crc_32(&self, size: u32, expected: u32) -> GetResult<()> {
        let bytes = self.peek_vec(u32_usize(size))?;
        let actual = crc_32(&bytes);
        if actual == expected {
            Ok(())
        } else {
            Err(GetError::ChecksumMismatch { expected, actual })
        }
    }
}

#[derive(Debug)]
struct Header {
    version: Version,
    label: Text,
    properties: Dictionary<Property>,
}

impl Get {
    fn get_header(&mut self) -> GetResult<Header> {
        let version = self.get_version()?;
        let label = self.get_text()?;
        let properties = self.get_dictionary(Self::get_property)?;
        Ok(Header {
            version,
            label,
            properties,
        })
    }
}

#[derive(Debug)]
struct Version {
    major: u32,
    minor: u32,
    patch: Option<u32>,
}

impl Get {
    fn get_version(&mut self) -> GetResult<Version> {
        let major = self.get_u32()?;
        let minor = self.get_u32()?;
        let patch = self.get_option((major, minor) >= (868, 18), Self::get_u32)?;
        Ok(Version {
            major,
            minor,
            patch,
        })
    }
}

#[derive(Debug)]
struct Text {
    size: i32,
    value: String,
}

impl Get {
    fn get_text(&mut self) -> GetResult<Text> {
        let size = self.get_i32()?;
        if size < 0 {
            let bytes = self.get_vec(i32_usize(-2 * size))?;
            match utf_16(&bytes) {
                None => Err(GetError::InvalidUtf16(bytes)),
                Some(value) => Ok(Text { size, value }),
            }
        } else {
            let size = if size == 0x0500_0000 { 8 } else { size };
            let bytes = self.get_vec(i32_usize(size))?;
            match windows_1252(&bytes) {
                None => Err(GetError::InvalidWindows1252(bytes)),
                Some(value) => Ok(Text { size, value }),
            }
        }
    }
}

#[derive(Debug)]
struct Dictionary<T> {
    value: Vec<(Text, T)>,
    last: Text,
}

impl Get {
    fn get_dictionary<F, T>(&mut self, get_value: F) -> GetResult<Dictionary<T>>
    where
        F: Fn(&mut Self) -> GetResult<T>,
    {
        let mut value = Vec::new();
        let last = loop {
            let k = self.get_text()?;
            if k.value.as_str() == "None\0" || k.value.as_str() == "\0\0\0None\0" {
                break k;
            }
            let v = get_value(self)?;
            value.push((k, v))
        };
        Ok(Dictionary { value, last })
    }
}

#[derive(Debug)]
struct Property {
    label: Text,
    size: u64,
    value: PropertyValue,
}

impl Get {
    fn get_property(&mut self) -> GetResult<Property> {
        let label = self.get_text()?;
        let size = self.get_u64()?;
        let value = self.get_property_value(label.value.as_str())?;
        Ok(Property { label, size, value })
    }
}

#[derive(Debug)]
enum PropertyValue {
    Array(List<Dictionary<Property>>),
    Bool(u8),
    Byte { key: Text, value: Option<Text> },
    Float(f32),
    Int(u32),
    Name(Text),
    QWord(u64),
    Str(Text),
}

impl Get {
    fn get_property_value(&mut self, label: &str) -> GetResult<PropertyValue> {
        match label {
            "ArrayProperty\0" => self.get_property_value_array(),
            "BoolProperty\0" => self.get_property_value_bool(),
            "ByteProperty\0" => self.get_property_value_byte(),
            "FloatProperty\0" => self.get_property_value_float(),
            "IntProperty\0" => self.get_property_value_int(),
            "NameProperty\0" => self.get_property_value_name(),
            "QWordProperty\0" => self.get_property_value_qword(),
            "StrProperty\0" => self.get_property_value_str(),
            _ => Err(GetError::UnknownProperty(String::from(label))),
        }
    }

    fn get_property_value_array(&mut self) -> GetResult<PropertyValue> {
        let x = self.get_list(|this| this.get_dictionary(|that| that.get_property()))?;
        Ok(PropertyValue::Array(x))
    }

    fn get_property_value_bool(&mut self) -> GetResult<PropertyValue> {
        let x = self.get_u8()?;
        Ok(PropertyValue::Bool(x))
    }

    fn get_property_value_byte(&mut self) -> GetResult<PropertyValue> {
        let key = self.get_text()?;
        let value = if key.value.as_str() == "OnlinePlatform_Steam\0" {
            Ok(None)
        } else {
            let x = self.get_text()?;
            Ok(Some(x))
        }?;
        Ok(PropertyValue::Byte { key, value })
    }

    fn get_property_value_float(&mut self) -> GetResult<PropertyValue> {
        let x = self.get_f32()?;
        Ok(PropertyValue::Float(x))
    }

    fn get_property_value_int(&mut self) -> GetResult<PropertyValue> {
        let x = self.get_u32()?;
        Ok(PropertyValue::Int(x))
    }

    fn get_property_value_name(&mut self) -> GetResult<PropertyValue> {
        let x = self.get_text()?;
        Ok(PropertyValue::Name(x))
    }

    fn get_property_value_qword(&mut self) -> GetResult<PropertyValue> {
        let x = self.get_u64()?;
        Ok(PropertyValue::QWord(x))
    }

    fn get_property_value_str(&mut self) -> GetResult<PropertyValue> {
        let x = self.get_text()?;
        Ok(PropertyValue::Str(x))
    }
}

#[derive(Debug)]
struct List<T> {
    size: u32,
    value: Vec<T>,
}

impl Get {
    fn get_list<F, T>(&mut self, get_value: F) -> GetResult<List<T>>
    where
        F: Fn(&mut Self) -> GetResult<T>,
    {
        let size = self.get_u32()?;
        let mut value = Vec::with_capacity(u32_usize(size));
        for _ in 0..size {
            let x = get_value(self)?;
            value.push(x)
        }
        Ok(List { size, value })
    }
}

#[derive(Debug)]
struct Content {
    levels: List<Text>,
    keyframes: List<Keyframe>,
    size: u32,
    messages: List<Message>,
    marks: List<Mark>,
    packages: List<Text>,
    objects: List<Text>,
    names: List<Text>,
    classes: List<Class>,
    caches: List<Cache>,
    frames: Vec<Frame>,
}

impl Get {
    fn get_content(&mut self, header: &Header) -> GetResult<Content> {
        let levels = self.get_list(Self::get_text)?;
        let keyframes = self.get_list(Self::get_keyframe)?;
        let size = self.get_u32()?;
        let bytes = self.get_vec(u32_usize(size))?;
        let messages = self.get_list(Self::get_message)?;
        let marks = self.get_list(Self::get_mark)?;
        let packages = self.get_list(Self::get_text)?;
        let objects = self.get_list(Self::get_text)?;
        let names = self.get_list(Self::get_text)?;
        let classes = self.get_list(Self::get_class)?;
        let caches = self.get_list(Self::get_cache)?;
        let frames = Self::get_frames(
            bytes,
            &mut Context::new(header, &names, &objects, &classes, &caches),
        )?;
        Ok(Content {
            levels,
            keyframes,
            size,
            messages,
            marks,
            packages,
            objects,
            names,
            classes,
            caches,
            frames,
        })
    }
}

#[derive(Debug)]
struct Keyframe {
    time: f32,
    frame: u32,
    offset: u32,
}

impl Get {
    fn get_keyframe(&mut self) -> GetResult<Keyframe> {
        let time = self.get_f32()?;
        let frame = self.get_u32()?;
        let offset = self.get_u32()?;
        Ok(Keyframe {
            time,
            frame,
            offset,
        })
    }
}

#[derive(Debug)]
struct Message {
    frame: u32,
    label: Text,
    value: Text,
}

impl Get {
    fn get_message(&mut self) -> GetResult<Message> {
        let frame = self.get_u32()?;
        let label = self.get_text()?;
        let value = self.get_text()?;
        Ok(Message {
            frame,
            label,
            value,
        })
    }
}

#[derive(Debug)]
struct Mark {
    value: Text,
    frame: u32,
}

impl Get {
    fn get_mark(&mut self) -> GetResult<Mark> {
        let value = self.get_text()?;
        let frame = self.get_u32()?;
        Ok(Mark { value, frame })
    }
}

#[derive(Debug)]
struct Class {
    name: Text,
    id: u32,
}

impl Get {
    fn get_class(&mut self) -> GetResult<Class> {
        let name = self.get_text()?;
        let id = self.get_u32()?;
        Ok(Class { name, id })
    }
}

#[derive(Debug)]
struct Cache {
    class: u32,
    parent: u32,
    index: u32,
    objects: List<Object>,
}

impl Get {
    fn get_cache(&mut self) -> GetResult<Cache> {
        let class = self.get_u32()?;
        let parent = self.get_u32()?;
        let index = self.get_u32()?;
        let objects = self.get_list(Self::get_object)?;
        Ok(Cache {
            class,
            parent,
            index,
            objects,
        })
    }
}

#[derive(Debug)]
struct Object {
    index: u32,
    id: u32,
}

impl Get {
    fn get_object(&mut self) -> GetResult<Object> {
        let index = self.get_u32()?;
        let id = self.get_u32()?;
        Ok(Object { index, id })
    }
}

struct Context {
    num_frames: usize,
    max_channels: u32,
    version: (u32, u32, u32),
    names: Vec<String>,
    objects: Vec<String>,
    classes: BTreeMap<u32, String>,
    classes_with_location: HashSet<&'static str>,
    classes_with_rotation: HashSet<&'static str>,
    actors: HashMap<u32, u32>,
    attributes: HashMap<u32, BTreeMap<u32, u32>>,
}

impl Context {
    fn new(
        header: &Header,
        names: &List<Text>,
        objects: &List<Text>,
        classes: &List<Class>,
        caches: &List<Cache>,
    ) -> Self {
        Context {
            num_frames: Self::get_num_frames(header),
            max_channels: Self::get_max_channels(header),
            version: Self::get_version(header),
            names: Self::get_names(names),
            objects: Self::get_objects(objects),
            classes: Self::get_classes(classes),
            classes_with_location: Self::get_classes_with_location(),
            classes_with_rotation: Self::get_classes_with_rotation(),
            actors: HashMap::new(),
            attributes: Self::get_attributes(caches),
        }
    }

    fn get_num_frames(header: &Header) -> usize {
        match header
            .properties
            .value
            .iter()
            .find(|property| property.0.value.as_str() == "NumFrames\0")
        {
            Some(&(
                _,
                Property {
                    value: PropertyValue::Int(num_frames),
                    ..
                },
            )) => u32_usize(num_frames),
            _ => 0,
        }
    }

    fn get_max_channels(header: &Header) -> u32 {
        match header
            .properties
            .value
            .iter()
            .find(|property| property.0.value.as_str() == "MaxChannels\0")
        {
            Some(&(
                _,
                Property {
                    value: PropertyValue::Int(max_channels),
                    ..
                },
            )) => max_channels,
            _ => 1_023,
        }
    }

    fn get_version(header: &Header) -> (u32, u32, u32) {
        (
            header.version.major,
            header.version.minor,
            header.version.patch.unwrap_or(0),
        )
    }

    fn get_names(names: &List<Text>) -> Vec<String> {
        names.value.iter().map(|name| name.value.clone()).collect()
    }

    fn get_objects(objects: &List<Text>) -> Vec<String> {
        objects
            .value
            .iter()
            .map(|object| object.value.clone())
            .collect()
    }

    fn get_classes(classes: &List<Class>) -> BTreeMap<u32, String> {
        classes
            .value
            .iter()
            .map(|class| (class.id, class.name.value.clone()))
            .collect()
    }

    fn get_classes_with_location() -> HashSet<&'static str> {
        [
            "TAGame.Ball_Breakout_TA\0",
            "TAGame.Ball_TA\0",
            "TAGame.CameraSettingsActor_TA\0",
            "TAGame.Car_Season_TA\0",
            "TAGame.Car_TA\0",
            "TAGame.CarComponent_Boost_TA\0",
            "TAGame.CarComponent_Dodge_TA\0",
            "TAGame.CarComponent_DoubleJump_TA\0",
            "TAGame.CarComponent_FlipCar_TA\0",
            "TAGame.CarComponent_Jump_TA\0",
            "TAGame.GameEvent_Season_TA\0",
            "TAGame.GameEvent_Soccar_TA\0",
            "TAGame.GameEvent_SoccarPrivate_TA\0",
            "TAGame.GameEvent_SoccarSplitscreen_TA\0",
            "TAGame.GRI_TA\0",
            "TAGame.PRI_TA\0",
            "TAGame.SpecialPickup_BallCarSpring_TA\0",
            "TAGame.SpecialPickup_BallFreeze_TA\0",
            "TAGame.SpecialPickup_BallGravity_TA\0",
            "TAGame.SpecialPickup_BallLasso_TA\0",
            "TAGame.SpecialPickup_BallVelcro_TA\0",
            "TAGame.SpecialPickup_Batarang_TA\0",
            "TAGame.SpecialPickup_BoostOverride_TA\0",
            "TAGame.SpecialPickup_GrapplingHook_TA\0",
            "TAGame.SpecialPickup_HitForce_TA\0",
            "TAGame.SpecialPickup_Swapper_TA\0",
            "TAGame.SpecialPickup_Tornado_TA\0",
            "TAGame.Team_Soccar_TA\0",
        ].iter()
            .cloned()
            .collect()
    }

    fn has_location(&self, class: &str) -> bool {
        self.classes_with_location.contains(class)
    }

    fn get_classes_with_rotation() -> HashSet<&'static str> {
        [
            "TAGame.Ball_Breakout_TA\0",
            "TAGame.Ball_TA\0",
            "TAGame.Car_Season_TA\0",
            "TAGame.Car_TA\0",
        ].iter()
            .cloned()
            .collect()
    }

    fn has_rotation(&self, class: &str) -> bool {
        self.classes_with_rotation.contains(class)
    }

    fn get_actor_class_id(&self, actor: u32) -> Option<u32> {
        match self.actors.get(&actor) {
            None => None,
            Some(&id) => Some(id),
        }
    }

    fn get_attributes(caches: &List<Cache>) -> HashMap<u32, BTreeMap<u32, u32>> {
        let mut class_index_to_class_id: VecDeque<(u32, u32)> = VecDeque::new();
        let mut class_id_to_parent_class_id = HashMap::new();
        let mut class_id_to_attributes = HashMap::new();

        for cache in &caches.value {
            let mut attributes: BTreeMap<u32, u32> = cache
                .objects
                .value
                .iter()
                .map(|x| (x.id, x.index))
                .collect();

            let parent = match class_index_to_class_id.iter().find(|x| x.0 == cache.parent) {
                Some(x) => {
                    class_id_to_parent_class_id.insert(cache.class, x.1);
                    Some(x.1)
                }
                None => match class_index_to_class_id.iter().find(|x| x.0 <= cache.parent) {
                    Some(x) => {
                        class_id_to_parent_class_id.insert(cache.class, x.1);
                        Some(x.1)
                    }
                    None => None,
                },
            };

            if let Some(parent_class_id) = parent {
                if let Some(parent_attributes) = class_id_to_attributes.get(&parent_class_id) {
                    attributes.extend(parent_attributes)
                }
            };

            class_id_to_attributes.insert(cache.class, attributes);

            class_index_to_class_id.push_front((cache.index, cache.class));
        }

        class_id_to_attributes
    }

    fn get_class_attributes(&self, class: u32) -> Option<BTreeMap<u32, u32>> {
        self.attributes.get(&class).cloned()
    }
}

#[derive(Debug)]
struct Frame {
    time: f32,
    delta: f32,
    replications: Vec<Replication>,
}

impl Get {
    fn get_frames(bytes: Vec<u8>, context: &mut Context) -> GetResult<Vec<Frame>> {
        match BitGet::new(bytes).get_frames(context) {
            Err(problem) => Err(GetError::BitGet(problem)),
            Ok(frames) => Ok(frames),
        }
    }
}

impl BitGet {
    fn get_frames(&mut self, context: &mut Context) -> BitGetResult<Vec<Frame>> {
        let mut frames = Vec::with_capacity(context.num_frames);
        for _ in 0..context.num_frames {
            let frame = self.get_frame(context)?;
            frames.push(frame)
        }
        Ok(frames)
    }

    fn get_frame(&mut self, context: &mut Context) -> BitGetResult<Frame> {
        let time = self.get_f32()?;
        let delta = self.get_f32()?;
        let replications = self.get_replications(context)?;
        Ok(Frame {
            time,
            delta,
            replications,
        })
    }
}

#[derive(Debug)]
struct Replication {
    actor: U32C,
    value: ReplicationValue,
}

impl BitGet {
    fn get_replications(&mut self, context: &mut Context) -> BitGetResult<Vec<Replication>> {
        let mut replications = Vec::new();
        loop {
            let has_more = self.get_bool()?;
            if has_more {
                let replication = self.get_replication(context)?;
                replications.push(replication)
            } else {
                break;
            }
        }
        Ok(replications)
    }

    fn get_replication(&mut self, context: &mut Context) -> BitGetResult<Replication> {
        let actor = self.get_u32c(context.max_channels)?;
        let value = self.get_replication_value(context, actor.value)?;
        if let ReplicationValue::Created { class_id, .. } = value {
            match context.actors.insert(actor.value, class_id) {
                _ => (),
            }
        }
        Ok(Replication { actor, value })
    }
}

#[derive(Debug)]
struct U32C {
    limit: u32,
    value: u32,
}

impl BitGet {
    fn get_u32c(&mut self, limit: u32) -> BitGetResult<U32C> {
        let mut value = 0;
        let max_index = (limit as f32).log2().ceil() as u32;
        let mut index = 0;
        loop {
            let step = 1 << index;
            let next_value = value + step;
            if index >= max_index || next_value > limit {
                break;
            }
            let flag = self.get_bool()?;
            if flag {
                value = next_value;
            }
            index += 1;
        }
        Ok(U32C { limit, value })
    }
}

#[derive(Debug)]
enum ReplicationValue {
    Created {
        unknown: bool,
        name_index: Option<u32>,
        name: Option<String>, // RO
        object_index: u32,
        object: String, // RO
        class_id: u32,  // RO
        class: String,  // RO
        location: Option<Location>,
        rotation: Option<Rotation>,
    },
    Updated(Vec<Attribute>),
    Destroyed,
}

impl BitGet {
    fn get_replication_value(
        &mut self,
        context: &Context,
        actor: u32,
    ) -> BitGetResult<ReplicationValue> {
        let is_open = self.get_bool()?;
        if is_open {
            let is_new = self.get_bool()?;
            if is_new {
                self.get_replication_value_created(context)
            } else {
                self.get_replication_value_updated(context, actor)
            }
        } else {
            self.get_replication_value_destroyed()
        }
    }

    fn get_replication_value_created(
        &mut self,
        context: &Context,
    ) -> BitGetResult<ReplicationValue> {
        let unknown = self.get_bool()?;
        let name_index = self.get_option(context.version >= (868, 14, 0), Self::get_u32)?;
        let name = match name_index {
            None => Ok(None),
            Some(index) => match context.names.get(u32_usize(index)) {
                None => Err(BitGetError::UnknownName(index)),
                Some(name) => Ok(Some(name.clone())),
            },
        }?;
        let object_index = self.get_u32()?;
        let object = match context.objects.get(u32_usize(object_index)) {
            None => Err(BitGetError::UnknownObject(object_index)),
            Some(name) => Ok(name.clone()),
        }?;
        let (class_id, class) = match context.classes.range(0..object_index).next_back() {
            None => Err(BitGetError::UnknownObjectClass(object.clone())),
            Some((&id, name)) => Ok((id, name.clone())),
        }?;
        let location = self.get_option(context.has_location(&class), Self::get_location)?;
        let rotation = self.get_option(context.has_rotation(&class), Self::get_rotation)?;
        Ok(ReplicationValue::Created {
            unknown,
            name_index,
            name,
            object_index,
            object,
            class_id,
            class,
            location,
            rotation,
        })
    }

    fn get_replication_value_updated(
        &mut self,
        context: &Context,
        actor: u32,
    ) -> BitGetResult<ReplicationValue> {
        let attributes = self.get_attributes(context, actor)?;
        Ok(ReplicationValue::Updated(attributes))
    }

    fn get_replication_value_destroyed(&mut self) -> BitGetResult<ReplicationValue> {
        Ok(ReplicationValue::Destroyed)
    }
}

#[derive(Debug)]
struct Location {
    size: U32C,
    x: U32C,
    y: U32C,
    z: U32C,
}

impl BitGet {
    fn get_location(&mut self) -> BitGetResult<Location> {
        let size = self.get_u32c(19)?;
        let limit = 4 << size.value;
        let x = self.get_u32c(limit)?;
        let y = self.get_u32c(limit)?;
        let z = self.get_u32c(limit)?;
        Ok(Location { size, x, y, z })
    }
}

#[derive(Debug)]
struct Rotation {
    x: Option<i8>,
    y: Option<i8>,
    z: Option<i8>,
}

impl BitGet {
    fn get_rotation(&mut self) -> BitGetResult<Rotation> {
        let has_x = self.get_bool()?;
        let x = self.get_option(has_x, Self::get_i8)?;
        let has_y = self.get_bool()?;
        let y = self.get_option(has_y, Self::get_i8)?;
        let has_z = self.get_bool()?;
        let z = self.get_option(has_z, Self::get_i8)?;
        Ok(Rotation { x, y, z })
    }
}

#[derive(Debug)]
struct Attribute {
    class_id: u32, // RO
    stream_id: U32C,
    object_id: u32, // RO
    object: String, // RO
    value: AttributeValue,
}

impl BitGet {
    fn get_attributes(&mut self, context: &Context, actor: u32) -> BitGetResult<Vec<Attribute>> {
        let mut attributes = Vec::new();
        loop {
            let has_more = self.get_bool()?;
            if has_more {
                let attribute = self.get_attribute(context, actor)?;
                attributes.push(attribute)
            } else {
                break;
            }
        }
        Ok(attributes)
    }

    fn get_attribute(&mut self, context: &Context, actor: u32) -> BitGetResult<Attribute> {
        let class_id = match context.get_actor_class_id(actor) {
            None => Err(BitGetError::UnknownActor(actor)),
            Some(id) => Ok(id),
        }?;
        let attributes = match context.get_class_attributes(class_id) {
            Some(x) => Ok(x),
            None => Err(BitGetError::UnknownClass(class_id)),
        }?;
        let limit = match attributes.keys().next_back() {
            Some(&x) => x,
            None => 0,
        };
        let stream_id = self.get_u32c(limit)?;
        let object_id = match attributes.get(&stream_id.value) {
            Some(&x) => Ok(x),
            None => Err(BitGetError::UnknownStreamId(stream_id.value)),
        }?;
        let object = match context.objects.get(u32_usize(object_id)) {
            Some(x) => Ok(x.clone()),
            None => Err(BitGetError::UnknownAttributeIndex(object_id)),
        }?;
        let value = self.get_attribute_value(&object)?;
        Ok(Attribute {
            class_id,
            stream_id,
            object_id,
            object,
            value,
        })
    }
}

#[derive(Debug)]
enum AttributeValue {}

impl BitGet {
    fn get_attribute_value(&mut self, name: &str) -> BitGetResult<AttributeValue> {
        // TODO
        Err(BitGetError::UnknownAttribute(String::from(name)))
    }
}

impl Get {
    fn get_f32(&mut self) -> GetResult<f32> {
        let x = self.get_u32()?;
        Ok(u32_f32(x))
    }

    fn get_i32(&mut self) -> GetResult<i32> {
        let x = self.get_u32()?;
        Ok(u32_i32(x))
    }

    fn get_option<F, T>(&mut self, condition: bool, get_value: F) -> GetResult<Option<T>>
    where
        F: Fn(&mut Self) -> GetResult<T>,
    {
        if condition {
            let value = get_value(self)?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn get_u8(&mut self) -> GetResult<u8> {
        let x = self.get_vec(1)?;
        Ok(x[0])
    }

    fn get_u16(&mut self) -> GetResult<u16> {
        let lower = self.get_u8()?;
        let upper = self.get_u8()?;
        Ok(u8_u16(lower) | u8_u16(upper) << 8)
    }

    fn get_u32(&mut self) -> GetResult<u32> {
        let lower = self.get_u16()?;
        let upper = self.get_u16()?;
        Ok(u16_u32(lower) | u16_u32(upper) << 16)
    }

    fn get_u64(&mut self) -> GetResult<u64> {
        let lower = self.get_u32()?;
        let upper = self.get_u32()?;
        Ok(u32_u64(lower) | u32_u64(upper) << 32)
    }

    fn get_vec(&mut self, len: usize) -> GetResult<Vec<u8>> {
        let bytes = self.peek_vec(len)?;
        self.index += len;
        Ok(bytes)
    }

    fn peek_vec(&self, len: usize) -> GetResult<Vec<u8>> {
        let index = self.index + len;
        match self.bytes.get(self.index..index) {
            None => Err(GetError::IndexOutOfBounds {
                index,
                len: self.bytes.len(),
            }),
            Some(bytes) => Ok(bytes.to_vec()),
        }
    }
}

impl BitGet {
    fn get_bool(&mut self) -> BitGetResult<bool> {
        match self.bytes.get(self.byte_index) {
            None => Err(BitGetError::IndexOutOfBounds {
                index: self.byte_index,
                len: self.bytes.len(),
            }),
            Some(byte) => {
                let bit = byte & 1 << self.bit_index != 0;
                self.bit_index += 1;
                if self.bit_index == 8 {
                    self.bit_index = 0;
                    self.byte_index += 1;
                }
                Ok(bit)
            }
        }
    }

    fn get_f32(&mut self) -> BitGetResult<f32> {
        let x = self.get_u32()?;
        Ok(u32_f32(x))
    }

    fn get_i8(&mut self) -> BitGetResult<i8> {
        let x = self.get_u8()?;
        Ok(u8_i8(x))
    }

    fn get_option<F, T>(&mut self, condition: bool, get_value: F) -> BitGetResult<Option<T>>
    where
        F: Fn(&mut Self) -> BitGetResult<T>,
    {
        if condition {
            let value = get_value(self)?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn get_u8(&mut self) -> BitGetResult<u8> {
        let a = self.get_bool()?;
        let b = self.get_bool()?;
        let c = self.get_bool()?;
        let d = self.get_bool()?;
        let e = self.get_bool()?;
        let f = self.get_bool()?;
        let g = self.get_bool()?;
        let h = self.get_bool()?;
        #[cfg_attr(rustfmt, rustfmt_skip)]
        Ok(
            if a { 0b0000_0001 } else { 0 } |
            if b { 0b0000_0010 } else { 0 } |
            if c { 0b0000_0100 } else { 0 } |
            if d { 0b0000_1000 } else { 0 } |
            if e { 0b0001_0000 } else { 0 } |
            if f { 0b0010_0000 } else { 0 } |
            if g { 0b0100_0000 } else { 0 } |
            if h { 0b1000_0000 } else { 0 },
        )
    }

    fn get_u16(&mut self) -> BitGetResult<u16> {
        let lower = self.get_u8()?;
        let upper = self.get_u8()?;
        Ok(u8_u16(lower) | u8_u16(upper) << 8)
    }

    fn get_u32(&mut self) -> BitGetResult<u32> {
        let lower = self.get_u16()?;
        let upper = self.get_u16()?;
        Ok(u16_u32(lower) | u16_u32(upper) << 16)
    }
}

fn i32_usize(x: i32) -> usize {
    x as usize
}

fn u8_i8(x: u8) -> i8 {
    x as i8
}

fn u8_u16(x: u8) -> u16 {
    u16::from(x)
}

fn u8_usize(x: u8) -> usize {
    x as usize
}

fn u16_u32(x: u16) -> u32 {
    u32::from(x)
}

fn u32_f32(x: u32) -> f32 {
    f32::from_bits(x)
}

fn u32_i32(x: u32) -> i32 {
    x as i32
}

fn u32_u8(x: u32) -> u8 {
    x as u8
}

fn u32_u64(x: u32) -> u64 {
    u64::from(x)
}

fn u32_usize(x: u32) -> usize {
    x as usize
}

fn crc_32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(0x1034_0dfe, |crc, byte| {
        crc << 8 ^ CRC_32[u8_usize(byte ^ u32_u8(crc >> 24))]
    })
}

const CRC_32: [u32; 256] = [
    0x0000_0000,
    0x04c1_1db7,
    0x0982_3b6e,
    0x0d43_26d9,
    0x1304_76dc,
    0x17c5_6b6b,
    0x1a86_4db2,
    0x1e47_5005,
    0x2608_edb8,
    0x22c9_f00f,
    0x2f8a_d6d6,
    0x2b4b_cb61,
    0x350c_9b64,
    0x31cd_86d3,
    0x3c8e_a00a,
    0x384f_bdbd,
    0x4c11_db70,
    0x48d0_c6c7,
    0x4593_e01e,
    0x4152_fda9,
    0x5f15_adac,
    0x5bd4_b01b,
    0x5697_96c2,
    0x5256_8b75,
    0x6a19_36c8,
    0x6ed8_2b7f,
    0x639b_0da6,
    0x675a_1011,
    0x791d_4014,
    0x7ddc_5da3,
    0x709f_7b7a,
    0x745e_66cd,
    0x9823_b6e0,
    0x9ce2_ab57,
    0x91a1_8d8e,
    0x9560_9039,
    0x8b27_c03c,
    0x8fe6_dd8b,
    0x82a5_fb52,
    0x8664_e6e5,
    0xbe2b_5b58,
    0xbaea_46ef,
    0xb7a9_6036,
    0xb368_7d81,
    0xad2f_2d84,
    0xa9ee_3033,
    0xa4ad_16ea,
    0xa06c_0b5d,
    0xd432_6d90,
    0xd0f3_7027,
    0xddb0_56fe,
    0xd971_4b49,
    0xc736_1b4c,
    0xc3f7_06fb,
    0xceb4_2022,
    0xca75_3d95,
    0xf23a_8028,
    0xf6fb_9d9f,
    0xfbb8_bb46,
    0xff79_a6f1,
    0xe13e_f6f4,
    0xe5ff_eb43,
    0xe8bc_cd9a,
    0xec7d_d02d,
    0x3486_7077,
    0x3047_6dc0,
    0x3d04_4b19,
    0x39c5_56ae,
    0x2782_06ab,
    0x2343_1b1c,
    0x2e00_3dc5,
    0x2ac1_2072,
    0x128e_9dcf,
    0x164f_8078,
    0x1b0c_a6a1,
    0x1fcd_bb16,
    0x018a_eb13,
    0x054b_f6a4,
    0x0808_d07d,
    0x0cc9_cdca,
    0x7897_ab07,
    0x7c56_b6b0,
    0x7115_9069,
    0x75d4_8dde,
    0x6b93_dddb,
    0x6f52_c06c,
    0x6211_e6b5,
    0x66d0_fb02,
    0x5e9f_46bf,
    0x5a5e_5b08,
    0x571d_7dd1,
    0x53dc_6066,
    0x4d9b_3063,
    0x495a_2dd4,
    0x4419_0b0d,
    0x40d8_16ba,
    0xaca5_c697,
    0xa864_db20,
    0xa527_fdf9,
    0xa1e6_e04e,
    0xbfa1_b04b,
    0xbb60_adfc,
    0xb623_8b25,
    0xb2e2_9692,
    0x8aad_2b2f,
    0x8e6c_3698,
    0x832f_1041,
    0x87ee_0df6,
    0x99a9_5df3,
    0x9d68_4044,
    0x902b_669d,
    0x94ea_7b2a,
    0xe0b4_1de7,
    0xe475_0050,
    0xe936_2689,
    0xedf7_3b3e,
    0xf3b0_6b3b,
    0xf771_768c,
    0xfa32_5055,
    0xfef3_4de2,
    0xc6bc_f05f,
    0xc27d_ede8,
    0xcf3e_cb31,
    0xcbff_d686,
    0xd5b8_8683,
    0xd179_9b34,
    0xdc3a_bded,
    0xd8fb_a05a,
    0x690c_e0ee,
    0x6dcd_fd59,
    0x608e_db80,
    0x644f_c637,
    0x7a08_9632,
    0x7ec9_8b85,
    0x738a_ad5c,
    0x774b_b0eb,
    0x4f04_0d56,
    0x4bc5_10e1,
    0x4686_3638,
    0x4247_2b8f,
    0x5c00_7b8a,
    0x58c1_663d,
    0x5582_40e4,
    0x5143_5d53,
    0x251d_3b9e,
    0x21dc_2629,
    0x2c9f_00f0,
    0x285e_1d47,
    0x3619_4d42,
    0x32d8_50f5,
    0x3f9b_762c,
    0x3b5a_6b9b,
    0x0315_d626,
    0x07d4_cb91,
    0x0a97_ed48,
    0x0e56_f0ff,
    0x1011_a0fa,
    0x14d0_bd4d,
    0x1993_9b94,
    0x1d52_8623,
    0xf12f_560e,
    0xf5ee_4bb9,
    0xf8ad_6d60,
    0xfc6c_70d7,
    0xe22b_20d2,
    0xe6ea_3d65,
    0xeba9_1bbc,
    0xef68_060b,
    0xd727_bbb6,
    0xd3e6_a601,
    0xdea5_80d8,
    0xda64_9d6f,
    0xc423_cd6a,
    0xc0e2_d0dd,
    0xcda1_f604,
    0xc960_ebb3,
    0xbd3e_8d7e,
    0xb9ff_90c9,
    0xb4bc_b610,
    0xb07d_aba7,
    0xae3a_fba2,
    0xaafb_e615,
    0xa7b8_c0cc,
    0xa379_dd7b,
    0x9b36_60c6,
    0x9ff7_7d71,
    0x92b4_5ba8,
    0x9675_461f,
    0x8832_161a,
    0x8cf3_0bad,
    0x81b0_2d74,
    0x8571_30c3,
    0x5d8a_9099,
    0x594b_8d2e,
    0x5408_abf7,
    0x50c9_b640,
    0x4e8e_e645,
    0x4a4f_fbf2,
    0x470c_dd2b,
    0x43cd_c09c,
    0x7b82_7d21,
    0x7f43_6096,
    0x7200_464f,
    0x76c1_5bf8,
    0x6886_0bfd,
    0x6c47_164a,
    0x6104_3093,
    0x65c5_2d24,
    0x119b_4be9,
    0x155a_565e,
    0x1819_7087,
    0x1cd8_6d30,
    0x029f_3d35,
    0x065e_2082,
    0x0b1d_065b,
    0x0fdc_1bec,
    0x3793_a651,
    0x3352_bbe6,
    0x3e11_9d3f,
    0x3ad0_8088,
    0x2497_d08d,
    0x2056_cd3a,
    0x2d15_ebe3,
    0x29d4_f654,
    0xc5a9_2679,
    0xc168_3bce,
    0xcc2b_1d17,
    0xc8ea_00a0,
    0xd6ad_50a5,
    0xd26c_4d12,
    0xdf2f_6bcb,
    0xdbee_767c,
    0xe3a1_cbc1,
    0xe760_d676,
    0xea23_f0af,
    0xeee2_ed18,
    0xf0a5_bd1d,
    0xf464_a0aa,
    0xf927_8673,
    0xfde6_9bc4,
    0x89b8_fd09,
    0x8d79_e0be,
    0x803a_c667,
    0x84fb_dbd0,
    0x9abc_8bd5,
    0x9e7d_9662,
    0x933e_b0bb,
    0x97ff_ad0c,
    0xafb0_10b1,
    0xab71_0d06,
    0xa632_2bdf,
    0xa2f3_3668,
    0xbcb4_666d,
    0xb875_7bda,
    0xb536_5d03,
    0xb1f7_40b4,
];

fn windows_1252(bytes: &[u8]) -> Option<String> {
    bytes
        .iter()
        .map(|&byte| WINDOWS_1252[u8_usize(byte)])
        .collect()
}

const WINDOWS_1252: [Option<char>; 256] = [
    // 0x0_
    Some('\u{0000}'), // null
    Some('\u{0001}'), // start of heading
    Some('\u{0002}'), // start of text
    Some('\u{0003}'), // end of text
    Some('\u{0004}'), // end of transmission
    Some('\u{0005}'), // enquiry
    Some('\u{0006}'), // acknowledge
    Some('\u{0007}'), // bell
    Some('\u{0008}'), // backspace
    Some('\u{0009}'), // horizontal tabulation
    Some('\u{000a}'), // line feed
    Some('\u{000b}'), // vertical tabulation
    Some('\u{000c}'), // form feed
    Some('\u{000d}'), // carriage return
    Some('\u{000e}'), // shift out
    Some('\u{000f}'), // shift in
    // 0x1_
    Some('\u{0010}'), // data link escape
    Some('\u{0011}'), // device control one
    Some('\u{0012}'), // device control two
    Some('\u{0013}'), // device control three
    Some('\u{0014}'), // device control four
    Some('\u{0015}'), // negative acknowledge
    Some('\u{0016}'), // synchronous idle
    Some('\u{0017}'), // end of transmission block
    Some('\u{0018}'), // cancel
    Some('\u{0019}'), // end of medium
    Some('\u{001a}'), // substitute
    Some('\u{001b}'), // escape
    Some('\u{001c}'), // file separator
    Some('\u{001d}'), // group separator
    Some('\u{001e}'), // record separator
    Some('\u{001f}'), // unit separator
    // 0x2_
    Some('\u{0020}'), // space
    Some('\u{0021}'), // exclamation mark
    Some('\u{0022}'), // quotation mark
    Some('\u{0023}'), // number sign
    Some('\u{0024}'), // dollar sign
    Some('\u{0025}'), // percent sign
    Some('\u{0026}'), // ampersand
    Some('\u{0027}'), // apostrophe
    Some('\u{0028}'), // left parenthesis
    Some('\u{0029}'), // right parenthesis
    Some('\u{002a}'), // asterisk
    Some('\u{002b}'), // plus sign
    Some('\u{002c}'), // comma
    Some('\u{002d}'), // hyphen-minus
    Some('\u{002e}'), // full stop
    Some('\u{002f}'), // solidus
    // 0x3_
    Some('\u{0030}'), // digit zero
    Some('\u{0031}'), // digit one
    Some('\u{0032}'), // digit two
    Some('\u{0033}'), // digit three
    Some('\u{0034}'), // digit four
    Some('\u{0035}'), // digit five
    Some('\u{0036}'), // digit six
    Some('\u{0037}'), // digit seven
    Some('\u{0038}'), // digit eight
    Some('\u{0039}'), // digit nine
    Some('\u{003a}'), // colon
    Some('\u{003b}'), // semicolon
    Some('\u{003c}'), // less-than sign
    Some('\u{003d}'), // equals sign
    Some('\u{003e}'), // greater-than sign
    Some('\u{003f}'), // question mark
    // 0x4_
    Some('\u{0040}'), // commercial at
    Some('\u{0041}'), // latin capital letter a
    Some('\u{0042}'), // latin capital letter b
    Some('\u{0043}'), // latin capital letter c
    Some('\u{0044}'), // latin capital letter d
    Some('\u{0045}'), // latin capital letter e
    Some('\u{0046}'), // latin capital letter f
    Some('\u{0047}'), // latin capital letter g
    Some('\u{0048}'), // latin capital letter h
    Some('\u{0049}'), // latin capital letter i
    Some('\u{004a}'), // latin capital letter j
    Some('\u{004b}'), // latin capital letter k
    Some('\u{004c}'), // latin capital letter l
    Some('\u{004d}'), // latin capital letter m
    Some('\u{004e}'), // latin capital letter n
    Some('\u{004f}'), // latin capital letter o
    // 0x5_
    Some('\u{0050}'), // latin capital letter p
    Some('\u{0051}'), // latin capital letter q
    Some('\u{0052}'), // latin capital letter r
    Some('\u{0053}'), // latin capital letter s
    Some('\u{0054}'), // latin capital letter t
    Some('\u{0055}'), // latin capital letter u
    Some('\u{0056}'), // latin capital letter v
    Some('\u{0057}'), // latin capital letter w
    Some('\u{0058}'), // latin capital letter x
    Some('\u{0059}'), // latin capital letter y
    Some('\u{005a}'), // latin capital letter z
    Some('\u{005b}'), // left square bracket
    Some('\u{005c}'), // reverse solidus
    Some('\u{005d}'), // right square bracket
    Some('\u{005e}'), // circumflex accent
    Some('\u{005f}'), // low line
    // 0x6_
    Some('\u{0060}'), // grave accent
    Some('\u{0061}'), // latin small letter a
    Some('\u{0062}'), // latin small letter b
    Some('\u{0063}'), // latin small letter c
    Some('\u{0064}'), // latin small letter d
    Some('\u{0065}'), // latin small letter e
    Some('\u{0066}'), // latin small letter f
    Some('\u{0067}'), // latin small letter g
    Some('\u{0068}'), // latin small letter h
    Some('\u{0069}'), // latin small letter i
    Some('\u{006a}'), // latin small letter j
    Some('\u{006b}'), // latin small letter k
    Some('\u{006c}'), // latin small letter l
    Some('\u{006d}'), // latin small letter m
    Some('\u{006e}'), // latin small letter n
    Some('\u{006f}'), // latin small letter o
    // 0x7_
    Some('\u{0070}'), // latin small letter p
    Some('\u{0071}'), // latin small letter q
    Some('\u{0072}'), // latin small letter r
    Some('\u{0073}'), // latin small letter s
    Some('\u{0074}'), // latin small letter t
    Some('\u{0075}'), // latin small letter u
    Some('\u{0076}'), // latin small letter v
    Some('\u{0077}'), // latin small letter w
    Some('\u{0078}'), // latin small letter x
    Some('\u{0079}'), // latin small letter y
    Some('\u{007a}'), // latin small letter z
    Some('\u{007b}'), // left curly bracket
    Some('\u{007c}'), // vertical line
    Some('\u{007d}'), // right curly bracket
    Some('\u{007e}'), // tilde
    Some('\u{007f}'), // delete
    // 0x8_
    Some('\u{20ac}'), // euro sign
    None,             // undefined
    Some('\u{201a}'), // single low-9 quotation mark
    Some('\u{0192}'), // latin small letter f with hook
    Some('\u{201e}'), // double low-9 quotation mark
    Some('\u{2026}'), // horizontal ellipsis
    Some('\u{2020}'), // dagger
    Some('\u{2021}'), // double dagger
    Some('\u{02c6}'), // modifier letter circumflex accent
    Some('\u{2030}'), // per mille sign
    Some('\u{0160}'), // latin capital letter s with caron
    Some('\u{2039}'), // single left-pointing angle quotation mark
    Some('\u{0152}'), // latin capital ligature oe
    None,             // undefined
    Some('\u{017d}'), // latin capital letter z with caron
    None,             // undefined
    // 0x9_
    None,             // undefined
    Some('\u{2018}'), // left single quotation mark
    Some('\u{2019}'), // right single quotation mark
    Some('\u{201c}'), // left double quotation mark
    Some('\u{201d}'), // right double quotation mark
    Some('\u{2022}'), // bullet
    Some('\u{2013}'), // en dash
    Some('\u{2014}'), // em dash
    Some('\u{02dc}'), // small tilde
    Some('\u{2122}'), // trade mark sign
    Some('\u{0161}'), // latin small letter s with caron
    Some('\u{203a}'), // single right-pointing angle quotation mark
    Some('\u{0153}'), // latin small ligature oe
    None,             // undefined
    Some('\u{017e}'), // latin small letter z with caron
    Some('\u{0178}'), // latin capital letter y with diaeresis
    // 0xa_
    Some('\u{00a0}'), // no-break space
    Some('\u{00a1}'), // inverted exclamation mark
    Some('\u{00a2}'), // cent sign
    Some('\u{00a3}'), // pound sign
    Some('\u{00a4}'), // currency sign
    Some('\u{00a5}'), // yen sign
    Some('\u{00a6}'), // broken bar
    Some('\u{00a7}'), // section sign
    Some('\u{00a8}'), // diaeresis
    Some('\u{00a9}'), // copyright sign
    Some('\u{00aa}'), // feminine ordinal indicator
    Some('\u{00ab}'), // left-pointing double angle quotation mark
    Some('\u{00ac}'), // not sign
    Some('\u{00ad}'), // soft hyphen
    Some('\u{00ae}'), // registered sign
    Some('\u{00af}'), // macron
    // 0xb_
    Some('\u{00b0}'), // degree sign
    Some('\u{00b1}'), // plus-minus sign
    Some('\u{00b2}'), // superscript two
    Some('\u{00b3}'), // superscript three
    Some('\u{00b4}'), // acute accent
    Some('\u{00b5}'), // micro sign
    Some('\u{00b6}'), // pilcrow sign
    Some('\u{00b7}'), // middle dot
    Some('\u{00b8}'), // cedilla
    Some('\u{00b9}'), // superscript one
    Some('\u{00ba}'), // masculine ordinal indicator
    Some('\u{00bb}'), // right-pointing double angle quotation mark
    Some('\u{00bc}'), // vulgar fraction one quarter
    Some('\u{00bd}'), // vulgar fraction one half
    Some('\u{00be}'), // vulgar fraction three quarters
    Some('\u{00bf}'), // inverted question mark
    // 0xc_
    Some('\u{00c0}'), // latin capital letter a with grave
    Some('\u{00c1}'), // latin capital letter a with acute
    Some('\u{00c2}'), // latin capital letter a with circumflex
    Some('\u{00c3}'), // latin capital letter a with tilde
    Some('\u{00c4}'), // latin capital letter a with diaeresis
    Some('\u{00c5}'), // latin capital letter a with ring above
    Some('\u{00c6}'), // latin capital letter ae
    Some('\u{00c7}'), // latin capital letter c with cedilla
    Some('\u{00c8}'), // latin capital letter e with grave
    Some('\u{00c9}'), // latin capital letter e with acute
    Some('\u{00ca}'), // latin capital letter e with circumflex
    Some('\u{00cb}'), // latin capital letter e with diaeresis
    Some('\u{00cc}'), // latin capital letter i with grave
    Some('\u{00cd}'), // latin capital letter i with acute
    Some('\u{00ce}'), // latin capital letter i with circumflex
    Some('\u{00cf}'), // latin capital letter i with diaeresis
    // 0xd_
    Some('\u{00d0}'), // latin capital letter eth
    Some('\u{00d1}'), // latin capital letter n with tilde
    Some('\u{00d2}'), // latin capital letter o with grave
    Some('\u{00d3}'), // latin capital letter o with acute
    Some('\u{00d4}'), // latin capital letter o with circumflex
    Some('\u{00d5}'), // latin capital letter o with tilde
    Some('\u{00d6}'), // latin capital letter o with diaeresis
    Some('\u{00d7}'), // multiplication sign
    Some('\u{00d8}'), // latin capital letter o with stroke
    Some('\u{00d9}'), // latin capital letter u with grave
    Some('\u{00da}'), // latin capital letter u with acute
    Some('\u{00db}'), // latin capital letter u with circumflex
    Some('\u{00dc}'), // latin capital letter u with diaeresis
    Some('\u{00dd}'), // latin capital letter y with acute
    Some('\u{00de}'), // latin capital letter thorn
    Some('\u{00df}'), // latin small letter sharp s
    // 0xe_
    Some('\u{00e0}'), // latin small letter a with grave
    Some('\u{00e1}'), // latin small letter a with acute
    Some('\u{00e2}'), // latin small letter a with circumflex
    Some('\u{00e3}'), // latin small letter a with tilde
    Some('\u{00e4}'), // latin small letter a with diaeresis
    Some('\u{00e5}'), // latin small letter a with ring above
    Some('\u{00e6}'), // latin small letter ae
    Some('\u{00e7}'), // latin small letter c with cedilla
    Some('\u{00e8}'), // latin small letter e with grave
    Some('\u{00e9}'), // latin small letter e with acute
    Some('\u{00ea}'), // latin small letter e with circumflex
    Some('\u{00eb}'), // latin small letter e with diaeresis
    Some('\u{00ec}'), // latin small letter i with grave
    Some('\u{00ed}'), // latin small letter i with acute
    Some('\u{00ee}'), // latin small letter i with circumflex
    Some('\u{00ef}'), // latin small letter i with diaeresis
    // 0xf_
    Some('\u{00f0}'), // latin small letter eth
    Some('\u{00f1}'), // latin small letter n with tilde
    Some('\u{00f2}'), // latin small letter o with grave
    Some('\u{00f3}'), // latin small letter o with acute
    Some('\u{00f4}'), // latin small letter o with circumflex
    Some('\u{00f5}'), // latin small letter o with tilde
    Some('\u{00f6}'), // latin small letter o with diaeresis
    Some('\u{00f7}'), // division sign
    Some('\u{00f8}'), // latin small letter o with stroke
    Some('\u{00f9}'), // latin small letter u with grave
    Some('\u{00fa}'), // latin small letter u with acute
    Some('\u{00fb}'), // latin small letter u with circumflex
    Some('\u{00fc}'), // latin small letter u with diaeresis
    Some('\u{00fd}'), // latin small letter y with acute
    Some('\u{00fe}'), // latin small letter thorn
    Some('\u{00ff}'), // latin small letter y with diaeresis
];

fn utf_16(bytes: &[u8]) -> Option<String> {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks(2) {
        let lower = u8_u16(chunk[0]);
        match chunk.get(1) {
            Some(&upper) => units.push(lower | u8_u16(upper) << 8),
            None => return None,
        }
    }
    match String::from_utf16(&units) {
        Ok(string) => Some(string),
        Err(_) => None,
    }
}
