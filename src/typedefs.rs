use std::string::String;
use std::{io, mem};
use std::cell::{OnceCell};
use std::fmt::{Debug, Display, Formatter};
use std::io::{Read};
use std::rc::Rc;
use crate::wire::*;
use crate::proto::{EnumProtoPtr, MessageProto, MessageProtoPtr};

#[derive(Default)]
pub struct CommonFieldProto {
    pub name: String,
    pub id: i32,
    pub repeated: bool,
    pub comment: String,
    pub oneof_name: Option<String>,
}


pub trait PbReaderTrait {
    fn pos(&self) -> usize;
    fn read_tag(&mut self, limit: &mut u32) -> io::Result<Tag>;
    fn read_varint(&mut self, limit: &mut u32) -> io::Result<i128>;
    fn read_len(&mut self, length: u32, limit: &mut u32) -> io::Result<Vec<u8>>;
}

pub struct PbReader<ReaderType: io::Read> {
    reader: ReaderType,
    pos: usize,
}

impl<ReaderType: io::Read> PbReader<ReaderType> {
    pub fn new(reader: ReaderType) -> PbReader<ReaderType> {
        PbReader { reader, pos: 0 }
    }
}
impl<ReaderType: io::Read> PbReaderTrait for PbReader<ReaderType> {
    fn pos(&self) -> usize {
        self.pos
    }
    fn read_tag(&mut self, limit: &mut u32) -> io::Result<Tag> {
        let first_number = self.read_varint(limit)? as i32;
        let length =
            match (first_number & 7) as u8 {
                WT_VARINT => 0,
                WT_I32 => 4,
                WT_I64 => 8,
                WT_LEN => self.read_varint(limit)? as u32,
                WT_SGROUP | WT_EGROUP =>
                    return Err(io::Error::new(io::ErrorKind::Unsupported, format!("Start/end group (deprecated) is not supported")).into()),
                other =>
                    return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Unsupported length type ({}) ", other)).into()),
            };
        Ok(Tag { first_number, length })
    }
    // read variable length integral value
    fn read_varint(&mut self, limit: &mut u32) -> io::Result<i128> {
        let mut buf: [u8; 1] = [0];
        let mut debug_str = String::new();
        let mut value: i128 = 0;
        let mut bits_read: u8 = 0;
        while 1 == self.reader.read(&mut buf)? {
            *limit -= 1;
            self.pos += 1;
            if 0 == (0x80u8 & buf[0]) {
                value = value | ((buf[0] as i128) << bits_read);
                return Ok(value);
            } else {
                if *limit == 0 { break; }
                value = value | (((buf[0] & 0x7fu8) as i128) << bits_read);
            }
            if bits_read > 64 - 8 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "VARINT overflow").into());
            }
            bits_read += 7;
        }
        Err(io::Error::new(io::ErrorKind::UnexpectedEof, "not completed VARINT"))
    }
    // read string or bytes with provided data length
    fn read_len(&mut self, length: u32, limit: &mut u32) -> io::Result<Vec<u8>> {
        if *limit >= length {
            *limit -= length as u32;
            let mut buf = vec![0u8; length as usize];
            self.reader.read_exact(&mut buf)?;
            self.pos += length as usize;
            Ok(buf)
        } else {
            Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read data out of limit"))
        }
    }
}


impl CommonFieldProto {
    // read integral or real value with predefined length
    fn read_fixed<const LEN: usize>(reader: &mut dyn PbReaderTrait, limit: &mut u32) -> io::Result<[u8; LEN]> {
        let mut buf = [0u8; LEN];
        let vec_buf = reader.read_len(LEN as u32, limit)?;
        for i in 0..buf.len() {
            buf[i] = vec_buf[i];
        }
        Ok(buf)
    }

    pub fn write_fixed<const N: usize>(writer: &mut dyn std::io::Write, data: &[u8; N]) -> io::Result<()> {
        writer.write_all(data)?;
        Ok(())
    }

    pub fn write_varint(writer: &mut dyn std::io::Write, data: i128) -> io::Result<()> {
        let mut data = data;
        let mut buf = vec![];
        buf.reserve(8);
        //while data & 0x80 != 0 { // > 0x7f {
        while (data as u128) > 0x7f {
            buf.push(((data as u8) & 0x7f) | 0x80);
            data = (data >> 7) & 0x7fffffffffffffff;
        }
        buf.push(data as u8);
        writer.write_all(&buf)
    }

    pub fn write_len(writer: &mut dyn std::io::Write, data: &[u8]) -> io::Result<()> {
        writer.write_all(&data)?;
        Ok(())
    }

    pub fn new_field(name: String, type_name: String, id: i32, repeated: bool, comment: String, oneof_name: Option<String>) -> Rc<dyn FieldProto> {
        let common = CommonFieldProto { name, id, repeated, comment, oneof_name };
        return
            match type_name.as_str() {
                "int32" => Rc::new(Int32FieldProto(common)),
                "uint32" => Rc::new(UInt32FieldProto(common)),
                "sint32" => Rc::new(SInt32FieldProto(common)),
                "fixed32" => Rc::new(FixedUInt32FieldProto(common)),
                "sfixed32" => Rc::new(FixedInt32FieldProto(common)),

                "int64" => Rc::new(Int64FieldProto(common)),
                "uint64" => Rc::new(UInt64FieldProto(common)),
                "sint64" => Rc::new(SInt64FieldProto(common)),
                "fixed64" => Rc::new(FixedUInt64FieldDefinition(common)),
                "sfixed64" => Rc::new(FixedInt64FieldDefinition(common)),

                "float" => Rc::new(FloatFieldDefinition(common)),
                "double" => Rc::new(DoubleFieldDefinition(common)),

                "bool" => Rc::new(BoolFieldDefinition(common)),

                "string" => Rc::new(StringFieldDefinition(common)),

                "bytes" => Rc::new(BytesFieldDefinition(common)),

                _ => Rc::new(EnumOrMessageFieldDefinition::new(common, type_name)),
            };
    }
}

pub trait FieldProto {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue>;
    // write only data, without field name and length
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()>;
    fn name(&self) -> String { self.get_common_definition().name.clone() }
    fn typename(&self) -> String;
    fn id(&self) -> i32 { self.get_common_definition().id }
    fn repeated(&self) -> bool { self.get_common_definition().repeated }
    fn wire_type(&self) -> u8 { WT_VARINT }
    fn oneof_name(&self) -> &Option<String> { &self.get_common_definition().oneof_name } // only if the field belongs to an oneof
    fn comment(&self) -> String { self.get_common_definition().comment.clone() }
    fn default(&self) -> FieldValue;
    fn get_common_definition(&self) -> &CommonFieldProto;
    //fn message_type_name(&self) -> &str { "" } // only if the field stores a message
    fn get_enum_name_by_index(&self, i: i32) -> Option<&str> { None }
    fn is_message(&self) -> bool { false }
    fn link_user_types(&self, _: &Vec<EnumProtoPtr>, _: &Vec<MessageProtoPtr>) {}
}

impl Debug for dyn FieldProto {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.typename().contains(",") {
            write!(f, "map<{}>", self.typename())?;
        } else {
            if self.repeated() { write!(f, "repeated ")? }
            write!(f, "{}", self.typename())?;
        }

        writeln!(f, " {} = {};", self.name(), self.id())
    }
}


pub struct Int32FieldProto(pub CommonFieldProto);

impl Int32FieldProto {
    pub const MIN: i32 = i32::MIN;
    pub const MAX: i32 = i32::MAX;
}
impl FieldProto for Int32FieldProto {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        let value = reader.read_varint(limit)? as i32;
        Ok(ScalarValue::I32(value))
    }

    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::I32(value) = data {
            CommonFieldProto::write_varint(writer, *value as i128)
        } else { unreachable!() }
    }
    fn typename(&self) -> String { "int32".to_string() }
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::I32(0)) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}

pub struct UInt32FieldProto(pub CommonFieldProto);
impl UInt32FieldProto {
    pub const MIN: u32 = u32::MIN;
    pub const MAX: u32 = u32::MAX;
}
impl FieldProto for UInt32FieldProto {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        let value = reader.read_varint(limit)? as u32;
        Ok(ScalarValue::U32(value))
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::U32(value) = data {
            CommonFieldProto::write_varint(writer, *value as i128)
        } else { unreachable!() }
    }
    fn typename(&self) -> String { "uint32".to_string() }
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::U32(0)) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}


pub struct SInt32FieldProto(pub CommonFieldProto);
impl SInt32FieldProto {
    pub const MIN: i32 = -0x7fff_ffff;
    pub const MAX: i32 = 0x7fff_ffff;
}

impl FieldProto for SInt32FieldProto {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        let zigzag = reader.read_varint(limit)?;
        let value = if 0 != (zigzag & 1) { -((zigzag >> 1) & 0x7fffffff) } else { (zigzag >> 1) & 0x7fffffff } as i32;
        Ok(ScalarValue::S32(value))
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::S32(value) = data {
            let zigzag = if *value >= 0 { *value << 1 } else { 1 + ((-*value) << 1) };
            return CommonFieldProto::write_varint(writer, zigzag as u32 as i128);
        }
        unreachable!()
    }
    fn typename(&self) -> String { "sint32".to_string() }
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::S32(0)) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}


pub struct FixedInt32FieldProto(pub CommonFieldProto);
impl FixedInt32FieldProto {
    pub const MIN: i32 = i32::MIN;
    pub const MAX: i32 = i32::MAX;
}
impl FieldProto for FixedInt32FieldProto {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        debug_assert_eq!(field_len, mem::size_of::<i32>() as u32);
        let bytes = CommonFieldProto::read_fixed(reader, limit)?;
        let value = i32::from_le_bytes(bytes);
        Ok(ScalarValue::SF32(value))
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::SF32(value) = data {
            CommonFieldProto::write_fixed(writer, &value.to_le_bytes())
        } else { unreachable!() }
    }
    fn typename(&self) -> String { "sfixed32".to_string() }
    fn wire_type(&self) -> u8 { WT_I32 }
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::SF32(0)) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}


pub struct FixedUInt32FieldProto(pub CommonFieldProto);
impl FixedUInt32FieldProto {
    pub const MIN: u32 = u32::MIN;
    pub const MAX: u32 = u32::MAX;
}
impl FieldProto for FixedUInt32FieldProto {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        debug_assert_eq!(field_len, mem::size_of::<u32>() as u32);
        let bytes = CommonFieldProto::read_fixed(reader, limit)?;
        let value = u32::from_le_bytes(bytes);
        Ok(ScalarValue::UF32(value))
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::UF32(value) = data {
            CommonFieldProto::write_fixed(writer, &value.to_le_bytes())
        } else { unreachable!() }
    }
    fn typename(&self) -> String { "fixed32".to_string() }
    fn wire_type(&self) -> u8 { WT_I32 }
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::UF32(0)) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}


pub struct Int64FieldProto(pub CommonFieldProto);
impl Int64FieldProto {
    pub const MIN: i64 = i64::MIN;
    pub const MAX: i64 = i64::MAX;
}
impl FieldProto for Int64FieldProto {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        let value = reader.read_varint(limit)? as i64;
        Ok(ScalarValue::I64(value))
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::I64(value) = data {
            CommonFieldProto::write_varint(writer, *value as i128)
        } else { unreachable!() }
    }
    fn typename(&self) -> String { "int64".to_string() }
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::I64(0)) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}


pub struct UInt64FieldProto(pub CommonFieldProto);
impl UInt64FieldProto {
    pub const MIN: u64 = u64::MIN;
    pub const MAX: u64 = u64::MAX;
}
impl FieldProto for UInt64FieldProto {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        let value = reader.read_varint(limit)? as u64;
        Ok(ScalarValue::U64(value))
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::U64(value) = data {
            CommonFieldProto::write_varint(writer, *value as i128)
        } else { unreachable!() }
    }
    fn typename(&self) -> String { "uint64".to_string() }
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::U64(0)) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}


pub struct SInt64FieldProto(pub CommonFieldProto);
impl SInt64FieldProto {
    pub const MIN: i64 = -0x7fff_ffff_ffff_ffff;
    pub const MAX: i64 = 0x7fff_ffff_ffff_ffff;
}
impl FieldProto for SInt64FieldProto {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        let zigzag = reader.read_varint(limit)?;
        let value = if 0 != (zigzag & 1) { -(zigzag >> 1) } else { zigzag >> 1 } as i64;
        Ok(ScalarValue::S64(value))
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::S64(value) = data {
            let zigzag = if *value >= 0 { *value << 1 } else { 1 + ((-*value) << 1) };
            return CommonFieldProto::write_varint(writer, zigzag as u64 as i128);
        }
        unreachable!()
    }
    fn typename(&self) -> String { "sint64".to_string() }
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::S64(0)) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}


pub struct FixedInt64FieldDefinition(pub CommonFieldProto);
impl FixedInt64FieldDefinition {
    pub const MIN: i64 = i64::MIN;
    pub const MAX: i64 = i64::MAX;
}
impl FieldProto for FixedInt64FieldDefinition {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        debug_assert_eq!(field_len, mem::size_of::<i64>() as u32);
        let bytes = CommonFieldProto::read_fixed(reader, limit)?;
        let value = i64::from_le_bytes(bytes);
        Ok(ScalarValue::SF64(value))
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::SF64(value) = data {
            CommonFieldProto::write_fixed(writer, &value.to_le_bytes())
        } else { unreachable!() }
    }
    fn typename(&self) -> String { "sfixed64".to_string() }
    fn wire_type(&self) -> u8 { WT_I64 }
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::SF64(0)) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}


pub struct FixedUInt64FieldDefinition(pub CommonFieldProto);
impl FixedUInt64FieldDefinition {
    pub const MIN: u64 = u64::MIN;
    pub const MAX: u64 = u64::MAX;
}
impl FieldProto for FixedUInt64FieldDefinition {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        debug_assert_eq!(field_len, mem::size_of::<u64>() as u32);
        let bytes = CommonFieldProto::read_fixed(reader, limit)?;
        let value = u64::from_le_bytes(bytes);
        Ok(ScalarValue::UF64(value))
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::UF64(value) = data {
            CommonFieldProto::write_fixed(writer, &value.to_le_bytes())
        } else { unreachable!() }
    }
    fn typename(&self) -> String { "fixed64".to_string() }
    fn wire_type(&self) -> u8 { WT_I64 }
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::UF64(0)) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}


pub struct FloatFieldDefinition(pub CommonFieldProto);
impl FieldProto for FloatFieldDefinition {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        debug_assert_eq!(field_len, mem::size_of::<f32>() as u32);
        let bytes = CommonFieldProto::read_fixed(reader, limit)?;
        let value = f32::from_le_bytes(bytes);
        Ok(ScalarValue::F32(value))
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::F32(value) = data {
            CommonFieldProto::write_fixed(writer, &value.to_le_bytes())
        } else { unreachable!() }
    }
    fn typename(&self) -> String { "float".to_string() }
    fn wire_type(&self) -> u8 { WT_I32 }
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::F32(0.0)) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}


pub struct DoubleFieldDefinition(pub CommonFieldProto);
impl FieldProto for DoubleFieldDefinition {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        debug_assert_eq!(field_len, mem::size_of::<f64>() as u32);
        let bytes = CommonFieldProto::read_fixed(reader, limit)?;
        let value = f64::from_le_bytes(bytes);
        Ok(ScalarValue::F64(value))
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::F64(value) = data {
            CommonFieldProto::write_fixed(writer, &value.to_le_bytes())
        } else { unreachable!() }
    }
    fn typename(&self) -> String { "double".to_string() }
    fn wire_type(&self) -> u8 { WT_I64 }
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::F64(0.0)) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}


pub struct BoolFieldDefinition(pub CommonFieldProto);
impl FieldProto for BoolFieldDefinition {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        let value = reader.read_varint(limit)?;
        Ok(ScalarValue::BOOL(value != 0))
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::BOOL(value) = data {
            return CommonFieldProto::write_varint(writer, *value as i128);
        }
        unreachable!()
    }
    fn typename(&self) -> String { "bool".to_string() }
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::BOOL(false)) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}


pub struct StringFieldDefinition(pub CommonFieldProto);
impl FieldProto for StringFieldDefinition {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        let buf = reader.read_len(field_len, limit)?;
        if let Ok(value) = String::from_utf8(buf) {
            Ok(ScalarValue::STR(value))
        } else {
            Ok(ScalarValue::STR("wrong unicode data".into()))
        }
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::STR(value) = data {
            return CommonFieldProto::write_len(writer, value.as_bytes());
        }
        unreachable!()
    }
    fn typename(&self) -> String { "string".to_string() }
    fn wire_type(&self) -> u8 { WT_LEN }
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::STR(String::new())) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}


pub struct BytesFieldDefinition(pub CommonFieldProto);
impl FieldProto for BytesFieldDefinition {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        Ok(ScalarValue::BYTES(reader.read_len(field_len, limit)?))
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::BYTES(value) = data {
            return CommonFieldProto::write_len(writer, value);
        }
        unreachable!()
    }
    fn typename(&self) -> String { "bytes".to_string() }
    fn wire_type(&self) -> u8 { WT_LEN }
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::BYTES(Vec::new())) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}


pub struct UnknownFieldDefinition(pub CommonFieldProto);
impl UnknownFieldDefinition {
    pub fn new() -> Self {
        Self(CommonFieldProto { name: "???".to_string(), id: 0, repeated: true, oneof_name: None, comment: String::new() })
    }

    pub fn read_unknown(reader: &mut dyn PbReaderTrait, limit: &mut u32, tlv: Tag) -> io::Result<ScalarValue> {
        if tlv.length == 0 {
            let value = reader.read_varint(limit)? as i64;
            let mut vec: Vec<u8> = value.to_le_bytes().into();
            while vec.last() == Some(&0) { // remove insignificant zeroes
                vec.pop();
            }
            Ok(ScalarValue::UNKNOWN(tlv, vec))
        } else {
            let buf = reader.read_len(tlv.length, limit)?;
            Ok(ScalarValue::UNKNOWN(tlv, buf))
        }
    }
}
impl FieldProto for UnknownFieldDefinition {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        unreachable!()
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::UNKNOWN(tlv, buf) = data {
            CommonFieldProto::write_varint(writer, tlv.first_number as i128)?;

            if tlv.wire_type() == WT_VARINT {
                let mut buf128 = [0u8; 16];
                for i in 0..=15 {
                    if i >= buf.len() { break; }
                    buf128[i] = buf[i];
                }
                let value = i128::from_le_bytes(buf128);
                return CommonFieldProto::write_varint(writer, value);
            } else {
                if !tlv.auto_length() { CommonFieldProto::write_varint(writer, tlv.length as i128)?; }
                return CommonFieldProto::write_len(writer, buf.as_slice());
            }
        }
        unreachable!()
    }
    fn typename(&self) -> String { "unknown".to_string() }
    fn wire_type(&self) -> u8 { panic!("wire type unknown"); } // depend on data read, but here is only type description
    fn default(&self) -> FieldValue { FieldValue::SCALAR(ScalarValue::UNKNOWN(Tag { first_number: 0, length: 0 }, Vec::new())) }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.0 }
}


pub struct EnumOrMessageFieldDefinition {
    pub common: CommonFieldProto,
    pub enum_proto: OnceCell<EnumProtoPtr>,
    pub is_message: OnceCell<MessageProtoPtr>,   // TODO rename
    pub typename: String,
}
impl EnumOrMessageFieldDefinition {
    pub fn new(common: CommonFieldProto, typename: String) -> Self {
        EnumOrMessageFieldDefinition {
            common,
            enum_proto: OnceCell::new(),
            is_message: OnceCell::new(),
            typename,
        }
    }
}
impl FieldProto for EnumOrMessageFieldDefinition {
    fn read(&self, reader: &mut dyn PbReaderTrait, limit: &mut u32, field_len: u32) -> io::Result<ScalarValue> {
        if let Some(_) = self.enum_proto.get() {
            let value = reader.read_varint(limit)? as i32;
            Ok(ScalarValue::ENUM(value))
        } else {
            panic!("read incomplete field definition {}", &self.common.name)
        }

        //if !self.variants.is_empty() {
        //    let value = reader.read_varint(limit)? as i32;
        //    Ok(ScalarValue::ENUM(value))
        //} else {
        //    panic!("read incomplete field definition {}", &self.common.name)
        //}
    }
    fn write(&self, writer: &mut dyn io::Write, data: &ScalarValue) -> io::Result<()> {
        if let ScalarValue::ENUM(value) = data {
            return CommonFieldProto::write_varint(writer, *value as i128);
        }
        unreachable!()
    }
    fn typename(&self) -> String { self.typename.clone() }
    fn wire_type(&self) -> u8 { if self.is_message.get().is_some() { WT_LEN } else { WT_VARINT } }
    fn default(&self) -> FieldValue {
        if let Some(def) = self.is_message.get() {
            FieldValue::MESSAGE(MessageData { def: def.clone(), fields: vec![] })
        } else {
            FieldValue::SCALAR(ScalarValue::ENUM(0))
        }
    }
    fn get_common_definition(&self) -> &CommonFieldProto { &self.common }
    fn is_message(&self) -> bool { self.is_message.get().is_some() }
    fn get_enum_name_by_index(&self, i: i32) -> Option<&str> {
        for v in &self.enum_proto.get()?.variants {
            if v.1 == i {
                return Some(&v.0);
            }
        }
        None
    }
    fn link_user_types(&self, enums: &Vec<EnumProtoPtr>, messages: &Vec<MessageProtoPtr>) {
        if let Ok(index) = messages.binary_search_by(|m| m.name.cmp(&self.typename)) {
            self.is_message.set(messages[index].clone()); //.unwrap();
            return;
        }
        if let Ok(index) = enums.binary_search_by(|m| m.name.cmp(&self.typename)) {
            self.enum_proto.set(enums[index].clone()).unwrap();
            return;
        }
        // TODO        panic!("unknown user type: {}", self.typename);
    }
}

