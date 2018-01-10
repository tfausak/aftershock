use std::env;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

fn main() {
    let arg = env::args().nth(1).unwrap();
    let path = Path::new(&arg);
    let mut file = File::open(path).unwrap();
    let mut bytes = Vec::new();
    let size = file.read_to_end(&mut bytes).unwrap();
    let mut decoder = Decoder::new(&bytes);
    let start = Instant::now();
    let result = decoder.get_replay();
    let elapsed = start.elapsed();
    let duration = (elapsed.as_secs() * 1_000_000) + u64::from(elapsed.subsec_nanos());
    println!(
        "{} {:>7} B {:>9} ns {:>8.1} B/s",
        match result {
            Ok(_) => "pass",
            Err(_) => "fail",
        },
        size,
        duration,
        (1_000_000 * size) as f64 / duration as f64,
    );
    println!("{:#?}", result);
}

#[derive(Debug)]
pub struct Replay {
    pub header: Section<Header>,
    pub content: Section<Content>,
}

impl Decoder {
    pub fn get_replay(&mut self) -> DecoderResult<Replay> {
        let header = self.get_section(Self::get_header)?;
        let content = self.get_section(Self::get_content)?;
        Ok(Replay { header, content })
    }
}

#[derive(Debug)]
pub struct Section<T> {
    pub size: u32,
    pub crc: u32,
    pub value: T,
}

impl Decoder {
    pub fn get_section<T>(&mut self, get_value: DecoderFn<T>) -> DecoderResult<Section<T>> {
        let size = self.get_u32()?;
        let crc = self.get_u32()?;
        self.check_crc(size as usize, crc)?;
        let value = get_value(self)?;
        Ok(Section { size, crc, value })
    }

    pub fn check_crc(&self, size: usize, crc: u32) -> DecoderResult<()> {
        let bytes = self.peek_bytes(size)?;
        let actual = compute_crc_32(&bytes);
        if actual != crc {
            Err(DecoderError::InvalidCrc {
                expected: crc,
                actual,
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Debug)]
pub struct Header {
    pub version: Version,
    pub label: Text,
    pub properties: Dictionary<Property>,
}

impl Decoder {
    pub fn get_header(&mut self) -> DecoderResult<Header> {
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
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: Option<u32>,
}

impl Decoder {
    pub fn get_version(&mut self) -> DecoderResult<Version> {
        let major = self.get_u32()?;
        let minor = self.get_u32()?;
        let patch = self.get_option(major >= 868 && minor >= 18, Self::get_u32)?;
        Ok(Version {
            major,
            minor,
            patch,
        })
    }
}

#[derive(Debug)]
pub struct Text {
    pub size: i32,
    pub value: String,
}

impl Decoder {
    pub fn get_text(&mut self) -> DecoderResult<Text> {
        let size = self.get_i32()?;
        let value = if size < 0 {
            self.get_utf_16((-2 * size) as usize)
        } else {
            self.get_windows_1252(size as usize)
        }?;
        Ok(Text { size, value })
    }

    pub fn get_utf_16(&mut self, size: usize) -> DecoderResult<String> {
        let bytes = self.get_bytes(size)?;
        match decode_utf_16(&bytes) {
            Some(string) => Ok(string),
            None => Err(DecoderError::InvalidUtf16(bytes)),
        }
    }

    pub fn get_windows_1252(&mut self, size: usize) -> DecoderResult<String> {
        let bytes = self.get_bytes(size)?;
        match decode_windows_1252(&bytes) {
            Some(string) => Ok(string),
            None => Err(DecoderError::InvalidWindows1252(bytes)),
        }
    }
}

#[derive(Debug)]
pub struct Dictionary<T> {
    pub value: Vec<(Text, T)>,
    pub last: Text,
}

impl Decoder {
    pub fn get_dictionary<T>(&mut self, get_value: DecoderFn<T>) -> DecoderResult<Dictionary<T>> {
        let mut value = Vec::new();
        let last = loop {
            let k = self.get_text()?;
            match k.value.as_str() {
                "None\0" => break k,
                _ => {
                    let v = get_value(self)?;
                    value.push((k, v))
                }
            }
        };
        Ok(Dictionary { value, last })
    }
}

#[derive(Debug)]
pub struct Property {
    pub label: Text,
    pub size: u64,
    pub value: PropertyValue<Property>,
}

impl Decoder {
    pub fn get_property(&mut self) -> DecoderResult<Property> {
        let label = self.get_text()?;
        let size = self.get_u64()?;
        let value = self.get_property_value(&label.value, Self::get_property)?;
        Ok(Property { label, size, value })
    }
}

#[derive(Debug)]
pub enum PropertyValue<T> {
    Array(List<Dictionary<T>>),
    Bool(u8),
    Byte(Text, Option<Text>),
    Float(f32),
    Int(u32),
    Name(Text),
    QWord(u64),
    Str(Text),
}

impl Decoder {
    pub fn get_property_value<T>(
        &mut self,
        label: &str,
        get_value: DecoderFn<T>,
    ) -> DecoderResult<PropertyValue<T>> {
        match label {
            "ArrayProperty\0" => self.get_property_value_array(get_value),
            "BoolProperty\0" => self.get_property_value_bool(),
            "ByteProperty\0" => self.get_property_value_byte(),
            "FloatProperty\0" => self.get_property_value_float(),
            "IntProperty\0" => self.get_property_value_int(),
            "NameProperty\0" => self.get_property_value_name(),
            "QWordProperty\0" => self.get_property_value_qword(),
            "StrProperty\0" => self.get_property_value_str(),
            _ => Err(DecoderError::UnknownProperty(String::from(label))),
        }
    }

    pub fn get_property_value_array<T>(
        &mut self,
        get_value: DecoderFn<T>,
    ) -> DecoderResult<PropertyValue<T>> {
        let x = self.get_list(|this| this.get_dictionary(get_value))?;
        Ok(PropertyValue::Array(x))
    }

    pub fn get_property_value_bool<T>(&mut self) -> DecoderResult<PropertyValue<T>> {
        let x = self.get_u8()?;
        Ok(PropertyValue::Bool(x))
    }

    pub fn get_property_value_byte<T>(&mut self) -> DecoderResult<PropertyValue<T>> {
        let key = self.get_text()?;
        let value = if key.value == "OnlinePlatform_Steam\0" {
            Ok(None)
        } else {
            let x = self.get_text()?;
            Ok(Some(x))
        }?;
        Ok(PropertyValue::Byte(key, value))
    }

    pub fn get_property_value_float<T>(&mut self) -> DecoderResult<PropertyValue<T>> {
        let x = self.get_f32()?;
        Ok(PropertyValue::Float(x))
    }

    pub fn get_property_value_int<T>(&mut self) -> DecoderResult<PropertyValue<T>> {
        let x = self.get_u32()?;
        Ok(PropertyValue::Int(x))
    }

    pub fn get_property_value_name<T>(&mut self) -> DecoderResult<PropertyValue<T>> {
        let x = self.get_text()?;
        Ok(PropertyValue::Name(x))
    }

    pub fn get_property_value_qword<T>(&mut self) -> DecoderResult<PropertyValue<T>> {
        let x = self.get_u64()?;
        Ok(PropertyValue::QWord(x))
    }

    pub fn get_property_value_str<T>(&mut self) -> DecoderResult<PropertyValue<T>> {
        let x = self.get_text()?;
        Ok(PropertyValue::Str(x))
    }
}

#[derive(Debug)]
pub struct List<T> {
    pub size: u32,
    pub value: Vec<T>,
}

impl Decoder {
    pub fn get_list<F, T>(&mut self, get_value: F) -> DecoderResult<List<T>>
    where
        F: Fn(&mut Self) -> DecoderResult<T>,
    {
        let size = self.get_u32()?;
        let mut value = Vec::with_capacity(size as usize);
        for _ in 0..size {
            let x = get_value(self)?;
            value.push(x)
        }
        Ok(List { size, value })
    }
}

#[derive(Debug)]
pub struct Content {
    pub levels: List<Text>,
    pub keyframes: List<Keyframe>,
    pub frames: Vec<Frame>,
    pub messages: List<Message>,
    pub marks: List<Mark>,
    pub packages: List<Text>,
    pub objects: List<Text>,
    pub names: List<Text>,
    pub classes: List<Class>,
    pub caches: List<Cache>,
}

impl Decoder {
    pub fn get_content(&mut self) -> DecoderResult<Content> {
        Err(DecoderError::NotImplemented)
    }
}

#[derive(Debug)]
pub struct Keyframe {
    pub time: f32,
    pub frame: u32,
    pub offset: u32,
}

#[derive(Debug)]
pub struct Frame {
    pub time: f32,
    pub delta: f32,
    pub replications: Vec<Replication>,
}

#[derive(Debug)]
pub struct Replication {
    pub actor: U32C,
    pub value: ReplicationValue,
}

#[derive(Debug)]
pub struct U32C {
    pub limit: u32,
    pub value: u32,
}

#[derive(Debug)]
pub enum ReplicationValue {
    Created {
        unknown: bool,
        name_index: Option<u32>,
        name: Option<Text>,
        object_id: u32,
        object: Text,
        class: Text,
        location: Option<Point<i32>>,
        rotation: Option<Point<Option<i8>>>,
    },
    Updated(Vec<Attribute>),
    Destroyed,
}

#[derive(Debug)]
pub struct Point<T> {
    pub x: T,
    pub y: T,
    pub z: T,
}

#[derive(Debug)]
pub struct Attribute {
    pub id: U32C,
    pub name: Text,
    pub value: AttributeValue,
}

#[derive(Debug)]
pub enum AttributeValue {
    AppliedDamage {
        unknown1: u8,
        location: Point<i32>,
        unknown2: i32,
        unknown3: i32,
    },
    Boolean(bool),
    Byte(u8),
    CamSettings {
        fov: f32,
        height: f32,
        angle: f32,
        distance: f32,
        stiffness: f32,
        swivel_speed: f32,
        transition_speed: Option<f32>,
    },
    ClubColors {
        unknown1: bool,
        blue: u8,
        unknown2: bool,
        orange: u8,
    },
    DamageState {
        unknown1: u8,
        unknown2: bool,
        unknown3: i32,
        unknown4: Point<i32>,
        unknown5: bool,
        unknown6: bool,
    },
    Demolish {
        unknown1: bool,
        attacker_actor: u32,
        unknown2: bool,
        victim_actor: u32,
        attacker_velocity: Point<i32>,
        victim_velocity: Point<i32>,
    },
    Enum(u16),
    Explosion(Explosion),
    ExtendedExplosion {
        explosion: Explosion,
        unknown: FlaggedInt,
    },
    FlaggedInt(FlaggedInt),
    Float(f32),
    GameMode {
        size: isize,
        value: u8,
    },
    Int(i32),
    Loadout(Loadout),
    LoadoutOnline(LoadoutOnline),
    Loadouts {
        blue: Loadout,
        orange: Loadout,
    },
    LoadoutsOnline {
        blue: LoadoutOnline,
        orange: LoadoutOnline,
        unknown1: bool,
        unknown2: bool,
    },
    Location(Point<i32>),
    MusicStinger {
        unknown: bool,
        cue: u32,
        trigger: u8,
    },
    PartyLeader {
        system: u8,
        id: Option<(RemoteId, u8)>,
    },
    Pickup {
        instigator: Option<u32>,
        picked_up: bool,
    },
    PlayerHistoryKey(Vec<bool>),
    PrivateMatchSettings {
        mutators: Text,
        joinable_by: u32,
        max_players: u32,
        game_name: Text,
        password: Text,
        unknown: bool,
    },
    QWord(u64),
    Reservation {
        number: U32C,
        id: UniqueId,
        name: Option<Text>,
        unknown1: bool,
        unknown2: bool,
        unknown3: Option<u8>,
    },
    RigidBodyState {
        unknown: bool,
        location: Point<i32>,
        rotation: Point<U32C>,
        linear_velocity: Option<Point<i32>>,
        angular_velocity: Option<Point<i32>>,
    },
    String(Text),
    TeamPaint {
        team: u8,
        primary_color: u8,
        accent_color: u8,
        primary_finish: u32,
        accent_finish: u32,
    },
    UniqueId(UniqueId),
    WeldedInfo {
        active: bool,
        actor: u32,
        offset: Point<i32>,
        mass: f32,
        rotation: Point<Option<i8>>,
    },
}

#[derive(Debug)]
pub struct Explosion {
    pub unknown: bool,
    pub actor: u32,
    pub location: Point<i32>,
}

#[derive(Debug)]
pub struct FlaggedInt {
    pub unknown: bool,
    pub value: i32,
}

#[derive(Debug)]
pub struct Loadout {
    pub version: u8,
    pub body: u32,
    pub decal: u32,
    pub wheels: u32,
    pub rocket_boost: u32,
    pub antenna: u32,
    pub topper: u32,
    pub unknown1: u32,
    pub unknown2: Option<u32>,
    pub engine: Option<u32>,
    pub trail: Option<u32>,
    pub goal: Option<u32>,
    pub banner: Option<u32>,
}

#[derive(Debug)]
pub struct LoadoutOnline {
    pub products: Vec<Vec<Product>>,
}

#[derive(Debug)]
pub struct Product {
    pub unknown: bool,
    pub object_id: u32,
    pub object: Option<Text>,
    pub value: ProductValue,
}

#[derive(Debug)]
pub enum ProductValue {
    PaintedOld(U32C),
    Painted(u32),
    UserColor(U32C),
}

#[derive(Debug)]
pub struct UniqueId {
    pub system: u8,
    pub remote: RemoteId,
    pub local: u8,
}

#[derive(Debug)]
pub enum RemoteId {
    Local(u32),
    PlayStation { name: Text, id: Vec<u8> },
    Steam(u64),
    Switch(Vec<bool>),
    Xbox(u64),
}

#[derive(Debug)]
pub struct Message {
    pub frame: u32,
    pub label: Text,
    pub value: Text,
}

#[derive(Debug)]
pub struct Mark {
    pub value: Text,
    pub frame: u32,
}

#[derive(Debug)]
pub struct Class {
    pub name: Text,
    pub id: u32,
}

#[derive(Debug)]
pub struct Cache {
    pub class: u32,
    pub parent: u32,
    pub id: u32,
    pub objects: List<Object>,
}

#[derive(Debug)]
pub struct Object {
    pub object: u32,
    pub id: u32,
}

//

pub struct Decoder {
    pub bytes: Vec<u8>,
    pub index: usize,
}

pub type DecoderFn<T> = fn(&mut Decoder) -> DecoderResult<T>;

pub type DecoderResult<T> = Result<T, DecoderError>;

#[derive(Debug)]
pub enum DecoderError {
    IndexOutOfBounds { index: usize, len: usize },
    InvalidCrc { expected: u32, actual: u32 },
    InvalidUtf16(Vec<u8>),
    InvalidWindows1252(Vec<u8>),
    NotImplemented,
    UnknownProperty(String),
}

impl Decoder {
    pub fn new(bytes: &[u8]) -> Self {
        Decoder {
            bytes: bytes.to_vec(),
            index: 0,
        }
    }

    pub fn get_option<T>(
        &mut self,
        condition: bool,
        get_value: DecoderFn<T>,
    ) -> DecoderResult<Option<T>> {
        if condition {
            let value = get_value(self)?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    pub fn get_f32(&mut self) -> DecoderResult<f32> {
        let x = self.get_u32()?;
        Ok(f32::from_bits(x))
    }

    pub fn get_i32(&mut self) -> DecoderResult<i32> {
        let x = self.get_u32()?;
        Ok(x as i32)
    }

    pub fn get_u64(&mut self) -> DecoderResult<u64> {
        let lower = self.get_u32()?;
        let upper = self.get_u32()?;
        Ok(u64::from(lower) | u64::from(upper) << 32)
    }

    pub fn get_u32(&mut self) -> DecoderResult<u32> {
        let lower = self.get_u16()?;
        let upper = self.get_u16()?;
        Ok(u32::from(lower) | u32::from(upper) << 16)
    }

    pub fn get_u16(&mut self) -> DecoderResult<u16> {
        let lower = self.get_u8()?;
        let upper = self.get_u8()?;
        Ok(u16::from(lower) | u16::from(upper) << 8)
    }

    pub fn get_u8(&mut self) -> DecoderResult<u8> {
        let bytes = self.get_bytes(1)?;
        Ok(bytes[0])
    }

    pub fn get_bytes(&mut self, size: usize) -> DecoderResult<Vec<u8>> {
        let bytes = self.peek_bytes(size)?;
        self.index += size;
        Ok(bytes)
    }

    pub fn peek_bytes(&self, size: usize) -> DecoderResult<Vec<u8>> {
        let end = self.index + size;
        match self.bytes.get(self.index..end) {
            Some(bytes) => Ok(bytes.to_vec()),
            None => Err(DecoderError::IndexOutOfBounds {
                index: end,
                len: self.bytes.len(),
            }),
        }
    }
}

pub fn compute_crc_32(bytes: &[u8]) -> u32 {
    let mut crc = 0x1034_0dfe;
    for byte in bytes {
        crc = (crc << 8) ^ CRC_32[(byte ^ ((crc >> 24) as u8)) as usize]
    }
    !crc
}

pub const CRC_32: [u32; 256] = [
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

pub fn decode_windows_1252(bytes: &[u8]) -> Option<String> {
    let mut string = String::with_capacity(bytes.len());
    for byte in bytes {
        match WINDOWS_1252[*byte as usize] {
            Some(c) => string.push(c),
            None => return None,
        }
    }
    Some(string)
}

// https://www.unicode.org/Public/MAPPINGS/VENDORS/MICSFT/WINDOWS/CP1252.TXT
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

pub fn decode_utf_16(bytes: &[u8]) -> Option<String> {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks(2) {
        let lower = u16::from(chunk[0]);
        match chunk.get(1) {
            Some(upper) => units.push(lower | u16::from(*upper) << 8),
            None => return None,
        }
    }
    match String::from_utf16(&units) {
        Ok(string) => Some(string),
        Err(_) => None,
    }
}
