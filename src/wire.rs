use std::fmt::{Debug, Display, Formatter};
use std::{io, mem};
use std::collections::HashMap;
use std::io::Read;
use crate::proto::*;
use crate::trz::{Change, ChangeType};
use crate::typedefs::*;
use crate::view::{FieldOrder, LayoutConfig, ScreenLine, IndentsCalc, TextStyle};

pub const WT_VARINT: u8 = 0;  // int32, int64, uint32, uint64, sint32, sint64, bool, enum
pub const WT_I64: u8 = 1;     // fixed64, sfixed64, double
pub const WT_LEN: u8 = 2;     // string, bytes, embedded messages, packed repeated fields
pub const WT_SGROUP: u8 = 3;  // is not supported
pub const WT_EGROUP: u8 = 4;  // is not supported
pub const WT_I32: u8 = 5;     // fixed32, sfixed32, float


#[derive(Debug, PartialEq, Clone)]
pub struct Tag
{
    pub first_number: i32,
    pub length: u32,
}

// stores only read data, no default value
pub struct MessageData {
    pub def: MessageProtoPtr,
    pub fields: Vec<FieldData>,
}

pub struct FieldData {
    pub def: FieldProtoPtr,
    pub pos: usize, // read position in file, or usize::MAX for new data
    pub value: FieldValue,
}

pub enum FieldValue {
    SCALAR(ScalarValue),
    MESSAGE(MessageData),
}

#[derive(Debug, PartialEq, Clone)]
pub enum ScalarValue {
    I32(i32),
    U32(u32),
    S32(i32),
    UF32(u32),
    SF32(i32),
    I64(i64),
    U64(u64),
    S64(i64),
    UF64(u64),
    SF64(i64),
    F32(f32),
    F64(f64),
    BOOL(bool),
    ENUM(i32),
    STR(String),
    BYTES(Vec<u8>),
    UNKNOWN(Tag, Vec<u8>), // tag into vec?
    // not field values, only for record changes
    DELETED,
    //    EMPTY, // a scalar without value or a message without fields
}

impl PartialEq for FieldValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (FieldValue::SCALAR(s1), FieldValue::SCALAR(s2)) => { s1 == s2 }
            (FieldValue::MESSAGE(s1), FieldValue::MESSAGE(s2)) => { unimplemented!() }
            _ => false,
        }
    }
}

// FieldData placement in a MessageData
#[derive(Debug, Clone, PartialEq)]
pub struct FieldPos {
    pub id: i32, // name(id) of field
    pub index: usize, // index is 0 unless field is repeated
}
pub struct FieldRange {
    pub id: i32, // name(id) of field
    pub index: usize, // start index
    pub amount: usize, // how many data items
}

//pub type FieldPath = Vec<FieldPos>;

#[derive(Debug, Clone, Default)]
pub struct FieldPath(pub Vec<FieldPos>);

impl FieldPath {
    pub fn new() -> FieldPath { FieldPath(vec![]) }
    pub fn push(&mut self, pos: FieldPos) { self.0.push(pos); }
    pub fn add(&self, pos: FieldPos) -> FieldPath {
        let mut items = self.0.clone();
        items.push(pos);
        FieldPath(items)
    }
    pub fn with_last_index(&self, index: usize) -> FieldPath {
        let mut items = self.0.clone();
        if let Some(last_item) = items.last_mut() {
            last_item.index = index;
        }
        FieldPath(items)
    }
}
impl<const size: usize> From<[(i32, usize); size]> for FieldPath {
    fn from(v: [(i32, usize); size]) -> FieldPath {
        let vector = v.into_iter().map(|item| FieldPos { id: item.0, index: item.1 }).collect();
        FieldPath(vector)
    }
}

impl From<(i32, usize)> for FieldPos {
    fn from(pos: (i32, usize)) -> Self { FieldPos { id: pos.0, index: pos.1 } }
}

impl From<FieldPos> for FieldRange {
    fn from(pos: FieldPos) -> Self {
        FieldRange {
            id: pos.id,
            index: pos.index,
            amount: 1,
        }
    }
}

impl Tag
{
    pub fn field_id(&self) -> i32 {
        self.first_number >> 3
    }
    pub fn wire_type(&self) -> u8 {
        (self.first_number & 7) as u8
    }
    pub fn auto_length(&self) -> bool {
        match (self.first_number & 7) as u8 {
            WT_VARINT | WT_I64 | WT_I32 => true,
            WT_LEN => false,
            _ => panic!()
        }
    }
}


impl Debug for MessageData {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        f.debug_struct("MessageData").
            field("name", &self.def.name).
            field("data len", &self.fields.len()).
            finish()
    }
}

impl FieldData {
    pub fn id(&self) -> i32 {
        match &self.value {
            FieldValue::SCALAR(ScalarValue::UNKNOWN(tag, _)) => { tag.field_id() }
            _ => self.def.id(),
        }
    }

    pub fn len(&self) -> usize {
        let data_size = match &self.value {
            FieldValue::SCALAR(scalar) => scalar.len(),
            FieldValue::MESSAGE(message) => message.len(),
        };
        ScalarValue::varint_size((self.def.id() as i128) << 3) + data_size
    }
}
impl Debug for FieldData {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        f.debug_struct("FieldData").
            field("name", &self.def.name()).
            field("value", &self.value).
            finish()
    }
}

impl Debug for FieldValue {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        match self {
            FieldValue::SCALAR(v) => {
                write!(f, "scalar: {}", v)
            }
            FieldValue::MESSAGE(v) => {
                write!(f, "submessage: {}", v)
            }
        }
    }
}

impl ScalarValue {
    pub fn varint_size(value: i128) -> usize {
        if value < 0 { todo!() }
        if value <= 0x00_0000_0000_0000_007f { return 1; }
        if value <= 0x00_0000_0000_0000_3fff { return 2; }
        if value <= 0x00_0000_0000_001f_ffff { return 3; }
        if value <= 0x00_0000_0000_0fff_ffff { return 4; }
        if value <= 0x00_0000_0007_ffff_ffff { return 5; }
        if value <= 0x00_0000_03ff_ffff_ffff { return 6; }
        if value <= 0x00_0001_ffff_ffff_ffff { return 7; }
        if value <= 0x00_00ff_ffff_ffff_ffff { return 8; }
        if value <= 0x00_7fff_ffff_ffff_ffff { return 9; }
        if value <= 0x3f_ffff_ffff_ffff_ffff { return 10; }
        panic!()
    }
    pub fn len(&self) -> usize {
        match self {
            ScalarValue::BOOL(_) => 1,
            ScalarValue::UF32(_) | ScalarValue::SF32(_) | ScalarValue::F32(_) => 4,
            ScalarValue::UF64(_) | ScalarValue::SF64(_) | ScalarValue::F64(_) => 8,
            ScalarValue::I32(v) => Self::varint_size(*v as i128),
            ScalarValue::S32(v) => Self::varint_size((*v as i128) << 1),
            ScalarValue::U32(v) => Self::varint_size(*v as i128),
            ScalarValue::U64(v) => Self::varint_size(*v as i128),
            ScalarValue::I64(v) => Self::varint_size(*v as i128),
            ScalarValue::S64(v) => Self::varint_size((*v as i128) << 1),
            ScalarValue::STR(v) => v.as_bytes().len(),
            ScalarValue::BYTES(v) => v.len(),
            ScalarValue::UNKNOWN(tag, bytes) => Self::varint_size(tag.first_number as i128) + bytes.len(),
            ScalarValue::ENUM(v) => Self::varint_size(*v as i128),
            ScalarValue::DELETED => 0,
        }
    }
}


impl Display for ScalarValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ScalarValue::DELETED => write!(f, "DELETED"),
            //ScalarValue::EMPTY => write!(f, "EMPTY"),
            ScalarValue::I32(v) => write!(f, "{}", v),
            ScalarValue::U32(v) => write!(f, "{}", v),
            ScalarValue::S32(v) => write!(f, "{}", v),
            ScalarValue::UF32(v) => write!(f, "{}", v),
            ScalarValue::SF32(v) => write!(f, "{}", v),
            ScalarValue::I64(v) => write!(f, "{}", v),
            ScalarValue::U64(v) => write!(f, "{}", v),
            ScalarValue::S64(v) => write!(f, "{}", v),
            ScalarValue::UF64(v) => write!(f, "{}", v),
            ScalarValue::SF64(v) => write!(f, "{}", v),
            ScalarValue::F32(v) => {
                let f1 = format!("{}", v);
                let f2 = format!("{:e}", v);
                write!(f, "{}", if f1.len() > f2.len() { f2 } else { f1 })
            }
            ScalarValue::F64(v) => {
                //write!(f, "{:e}", v)
                let f1 = format!("{}", v);
                let f2 = format!("{:e}", v);
                write!(f, "{}", if f1.len() > f2.len() { f2 } else { f1 })
            }
            ScalarValue::BOOL(v) => write!(f, "{}", v),
            ScalarValue::STR(v) => write!(f, "{}", v),
            ScalarValue::BYTES(v) => {
                let mut s = String::new();
                for b in v { s += format!("{:02x} ", b).as_str(); }
                write!(f, "{}", s.trim_end())
            }
            ScalarValue::UNKNOWN(tag, bytes) => {
                let mut s = format!("{}.{}: ", tag.field_id(), tag.length);
                for b in bytes { s += format!("{:02x} ", b).as_str(); }
                write!(f, "{}", s.trim_end())
            }
            ScalarValue::ENUM(_) => panic!("cannot display enum directly"),
        }
    }
}


impl<'proto> MessageData {
    pub fn new(reader: &mut dyn PbReaderTrait, proto: &'proto ProtoData, def: MessageProtoPtr, limit: &mut u32) -> io::Result<Self> {
        let mut flds = Vec::<(FieldProtoPtr, usize, FieldValue)>::new();
        while *limit > 0 {
            let mut tag = reader.read_tag(limit)?;
            match def.get_field(tag.field_id()) {
                Some(field_def) => { // read sumbessage field
                    if field_def.is_message() {
                        *limit -= tag.length;
                        let submsg_def = proto.get_message_definition(&field_def.typename()).unwrap();
                        flds.push((field_def, reader.pos(), FieldValue::MESSAGE(MessageData::new(reader, proto, submsg_def, &mut tag.length)?)));
                    } else {
                        if !field_def.repeated() {
                            flds.push((field_def.clone(), reader.pos(), FieldValue::SCALAR(field_def.read(reader, limit, tag.length)?)));
                        } else {
                            if tag.auto_length() || field_def.wire_type() == WT_LEN { // not packable
                                flds.push((field_def.clone(), reader.pos(), FieldValue::SCALAR(field_def.read(reader, limit, tag.length)?)));
                            } else {
                                while *limit > 0 {
                                    flds.push((field_def.clone(), reader.pos(), FieldValue::SCALAR(field_def.read(reader, limit, tag.length)?)));
                                }
                            }
                        }
                    }
                }
                None => { // field id not found in the message definition
                    flds.push((proto.unknown_field.clone(), reader.pos(), FieldValue::SCALAR(UnknownFieldDefinition::read_unknown(reader, limit, tag)?)));
                }
            }
        }

        // remove duplicated fields
        // if an oneof or non-repeated field duplicated we should not remove old values,
        // instead we should save it and show the errors to the user
        //let to_delete = Self::find_duplicated_fields(&mut flds);

        let fields = flds.into_iter().enumerate().
            // TODO filter(|m| !to_delete.contains(&m.0)).
            map(|m| FieldData { def: m.1.0, pos: m.1.1, value: m.1.2 }).
            collect();

        Ok(MessageData { fields, def })
    }

    //fn find_duplicated_fields(fields: &Vec::<(&dyn FieldDefinition, usize, FieldValue)>) -> HashSet<usize> {
    //    let mut ignore = vec![];
    //    if !fields.is_empty() {
    //        let mut i = fields.len() - 1;
    //        loop {
    //            if i == 0 { break; } else {
    //                if !fields[i].0.repeated() {
    //                    let id = fields[i].0.id();
    //                    let mut j = i as i32 - 1;
    //                    loop {
    //                        if j < 0 { break; } else {
    //                            if fields[j as usize].0.id() == id {
    //                                ignore.push(j as usize);
    //                            }
    //                            j -= 1;
    //                        }
    //                    }
    //                }
    //
    //                if let Some(oneof_name) = fields[i].0.oneof_name() {
    //                    let mut j = i as i32 - 1;
    //                    loop {
    //                        if j < 0 { break; } else {
    //                            if let Some(ofn) = fields[j as usize].0.oneof_name() {
    //                                if oneof_name == ofn {
    //                                    ignore.push(j as usize);
    //                                }
    //                            }
    //                            j -= 1;
    //                        }
    //                    }
    //                }
    //
    //                i -= 1;
    //            }
    //        }
    //    }
    //    ignore.into_iter().collect()
    //}

    // data written as it was read
    pub fn write(&self, writer: &mut dyn io::Write, proto: &'proto ProtoData, _def: MessageProtoPtr) -> io::Result<()> {
        for field in &self.fields {
            if let FieldValue::SCALAR(ScalarValue::UNKNOWN(tag, data)) = &field.value {
                if let FieldValue::SCALAR(scalar) = &field.value {
                    field.def.write(writer, scalar)?;
                }
            } else {
                // write field index and wire type
                CommonFieldProto::write_varint(writer, ((field.def.id() << 3) | field.def.wire_type() as i32) as i128)?;
                if field.def.wire_type() != WT_LEN {
                    if let FieldValue::SCALAR(scalar) = &field.value { // write scalar with known length
                        field.def.write(writer, scalar)?;
                    }
                } else {
                    // variable length data. First write to the temporary buffer to measure the length
                    let mut buf = vec![];
                    match &field.value {
                        FieldValue::MESSAGE(msg) => { msg.write(&mut buf, proto, msg.def.clone())? }
                        FieldValue::SCALAR(scalar) => { field.def.write(&mut buf, scalar)? }
                    }
                    CommonFieldProto::write_varint(writer, buf.len() as i128)?;
                    CommonFieldProto::write_len(writer, &buf)?;
                }
            }
        }
        Ok(())
    }

    pub fn get_field<'x, 'y: 'x>(&'y self, path: &[FieldPos]) -> Option<&'x FieldData> {
        if let Some((first, others)) = path.split_last() {
            let msg = self.get_submessage(others)?;
            let pos = msg.get_field_pos(first.id, first.index)?;
            Some(&msg.fields[pos])
        } else { None }
    }
    pub fn get_field_mut<'x, 'y: 'x>(&'y mut self, path: &[FieldPos]) -> Option<&'x mut FieldData> {
        if let Some((first, others)) = path.split_last() {
            let msg = self.get_submessage_mut(others)?;
            let pos = msg.get_field_pos(first.id, first.index)?;
            Some(&mut msg.fields[pos])
        } else { None }
    }
    pub fn add_field<'x, 'y: 'x>(&'y mut self, path: &[FieldPos]) -> Option<&'x mut FieldData> {
        if let Some((first, others)) = path.split_last() {
            self.get_submessage_mut(others)?.add_field_private(first.id, first.index)
        } else { None }
    }
    pub fn delete_field<'x, 'y: 'x>(&'y mut self, path: &[FieldPos]) -> Option<FieldValue> {
        if let Some((first, others)) = path.split_last() {
            self.get_submessage_mut(others)?.delete_field_private(first.id, first.index)
        } else { None }
    }
    fn add_field_private<'x, 'y: 'x>(&'y mut self, id: i32, index: usize) -> Option<&'x mut FieldData> {
        if let Some(def) = self.def.fields.iter().find(|f| f.id() == id) {
            let insert_pos = if let Some(pos) = self.get_field_pos(id, index) { pos } else { self.fields.len() };
            self.fields.insert(insert_pos, FieldData { def: def.clone(), pos: usize::MAX, value: def.default() });
            Some(&mut self.fields[insert_pos])
        } else { None }
    }
    fn delete_field_private(&mut self, id: i32, index: usize) -> Option<FieldValue> {
        if let Some(del_pos) = self.get_field_pos(id, index) {
            Some(self.fields.remove(del_pos).value)
        } else { None }
    }
    pub fn get_submessage_mut<'x, 'y: 'x>(&'y mut self, path: &[FieldPos]) -> Option<&'x mut MessageData> {
        if path.is_empty() {
            Some(self)
        } else {
            let split = path.split_first().unwrap();
            if let Some(pos) = self.get_field_pos(split.0.id, split.0.index) {
                if let FieldValue::MESSAGE(msg) = &mut self.fields[pos].value {
                    return if split.1.is_empty() {
                        Some(msg)
                    } else {
                        msg.get_submessage_mut(split.1)
                    };
                }
            }
            None
        }
    }
    pub fn get_submessage<'x, 'y: 'x>(&'y self, path: &[FieldPos]) -> Option<&'x MessageData> {
        if path.is_empty() {
            Some(self)
        } else {
            let split = path.split_first().unwrap();
            if let Some(pos) = self.get_field_pos(split.0.id, split.0.index) {
                if let FieldValue::MESSAGE(msg) = &self.fields[pos].value {
                    return if split.1.is_empty() {
                        Some(msg)
                    } else {
                        msg.get_submessage(split.1)
                    };
                }
            }
            None
        }
    }
    fn get_field_pos(&self, id: i32, mut index: usize) -> Option<usize> {
        let pos = self.fields.iter().position(|f|
            if f.id() == id { // search for nth value with id matched
                if index == 0 { true } else {
                    index -= 1;
                    false
                }
            } else { false });
        if index != 0 { return None; }
        pos
    }

    // can find field definition even if the field was not read (only exist in proto file)
    pub fn get_field_definition(&self, path: &FieldPath) -> Option<FieldProtoPtr> {
        let mut p = path.0.clone();
        if let Some(last_path_item) = p.pop() {
            if let Some(parent) = self.get_submessage(&p.as_slice()) {
                return parent.def.get_field(last_path_item.id);
            }
        }
        None
    }


    pub fn get_sorted_fields(&self, order: &FieldOrder) -> Vec<(FieldPos, usize)> {


        // assert_eq!(order, &FieldOrder::Proto);

        if *order == FieldOrder::Wire {
            let mut indexes = HashMap::<i32, usize>::new();
            let fields_positions: Vec<FieldPos> =
                self.fields.iter().map(|field| {
                    let id = field.id();
                    return if let Some(i) = indexes.get_mut(&id) {
                        *i += 1;
                        FieldPos { id, index: *i }
                    } else {
                        indexes.insert(id, 0);
                        FieldPos { id, index: 0 }
                    };
                }).collect();


            // union field sequences with equal id
            let mut it = fields_positions.into_iter().peekable();
            let mut current = FieldPos { id: 0, index: 0 };
            let mut amount = 0;
            let mut res: Vec<(FieldPos, usize)> = vec![];
            loop {
                if let Some(value) = it.next() {
                    if amount == 0 {
                        current.id = value.id;
                        current.index = value.index;
                    }
                    amount += 1;

                    let mut next_the_same = false;
                    if let Some(next) = it.peek() {
                        if next.id == value.id {
                            next_the_same = true;
                        }
                    }
                    if !next_the_same {
                        res.push((current.clone(), amount));
                        amount = 0;
                    }
                } else { break; }
            }
            return res;
        }

        let mut fdefs = self.def.fields.clone();
        if *order != FieldOrder::Proto {
            fdefs.sort_by(|def1, def2| {
                match order {
                    FieldOrder::ByName => def1.name().cmp(&def2.name()),
                    FieldOrder::ById => def1.id().cmp(&def2.id()),
                    FieldOrder::Wire | FieldOrder::Proto => unreachable!()
                }
            });
        }

        let mut res: Vec<(FieldPos, usize)> = vec![];
        for fd in fdefs {
            let amount = self.fields.iter().
                map(|f| (f.id() == fd.id()) as usize).
                reduce(|acc, v| acc + v).
                unwrap_or_default();

            res.push((FieldPos { id: fd.id(), index: 0 }, amount));
        }
        return res;
    }

    pub fn apply(&mut self, change: &mut Change) -> Option<()> {
        match &mut change.action {
            //            ChangeType::Overwrite(value) => {
            //                let field =
            //                    if let Some(exist_field) = self.get_field_mut(&change.path) {
            //                        exist_field
            //                    } else {
            //                        // if there is no field, create it with default value
            //                        // undo threat the default value as no field
            //                        self.add_field(&change.path)?
            //                    };
            //                mem::swap(&mut field.value, value);
            //            }

            ChangeType::Overwrite(value) => {
                let field = self.get_field_mut(&change.path.0)?;
                mem::swap(&mut field.value, value);
            }

            ChangeType::Insert(value) => {
                let field = self.add_field(&change.path.0)?;
                mem::swap(&mut field.value, value);
                change.action = ChangeType::Delete;
            }

            //            ChangeType::Insert => {
            //                self.add_field(&change.path)?;
            //                change.action = ChangeType::Delete;
            //            }

            ChangeType::Delete => {
                change.action = ChangeType::Insert(self.delete_field(&change.path.0)?)
            }
        }
        Some(())
    }

    // result may be inaccurate in case of a packed field (todo)
    pub fn len(&self) -> usize {
        self.fields.iter().fold(0, |acc, field| acc + field.len())
    }
}

impl std::fmt::Display for MessageData {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "message {} {{", &self.def.name)?;
        for field in &self.fields {
            write!(f, "  {} = ", field.def.name())?;
            match &field.value {
                FieldValue::SCALAR(scalar) => {
                    if let ScalarValue::ENUM(index) = scalar {
                        if let Some(item_name) = field.def.get_enum_name_by_index(*index) {
                            writeln!(f, "{}", item_name.to_string())?;
                        }
                    } else {
                        writeln!(f, "{}", scalar)?;
                    }
                }
                FieldValue::MESSAGE(msg) => {
                    writeln!(f, "{}", msg.to_string())?;
                }
            }
        }
        writeln!(f, "}}")?;
        Ok(())
    }
}

/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
/***************************************************************************************************/
#[cfg(test)]
mod scalars {
    use super::*;

    struct TestData {
        value: i128,
        bytes: Vec<u8>,
        limit: u32,
    }

    fn ok_data() -> [TestData; 7] {
        [
            TestData { value: 0, bytes: vec![0], limit: u32::MAX },
            TestData { value: 0x55, bytes: vec![0x55], limit: u32::MAX },
            TestData { value: 0x5555, bytes: vec![0xd5, 0xaa, 0x01], limit: u32::MAX },
            TestData { value: 150, bytes: vec![0x96, 0x01], limit: u32::MAX },
            TestData { value: 0x7fffffffffffffff, bytes: vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f], limit: u32::MAX },
            TestData { value: 0x55, bytes: vec![0x55], limit: 1 },
            TestData { value: 0x5555, bytes: vec![0xd5, 0xaa, 0x01], limit: 3 },
        ]
    }

    fn wrong_data() -> [TestData; 3] {
        [
            TestData { value: 0, bytes: vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0], limit: u32::MAX },
            TestData { value: 0, bytes: vec![0xff; 3], limit: u32::MAX },
            TestData { value: 0, bytes: vec![0x96, 0x01], limit: 1 },
        ]
    }


    #[test]
    fn read_varint() {
        for data in &ok_data() {
            let mut limit = data.limit;
            let mut io_read = data.bytes.as_slice();
            let mut read = PbReader::new(&mut io_read);
            assert_eq!(data.value, read.read_varint(&mut limit).unwrap());
            assert_eq!(data.bytes.len(), ScalarValue::varint_size(data.value));
        }

        for data in &wrong_data() {
            let mut limit = data.limit;
            let mut io_read = data.bytes.as_slice();
            let mut read = PbReader::new(&mut io_read);
            assert!(read.read_varint(&mut limit).is_err());
        }
    }
    #[test]
    fn write_varint() {
        for data in &ok_data() {
            let mut buf = vec![];
            assert!(CommonFieldProto::write_varint(&mut buf, data.value).is_ok());
            assert_eq!(buf, data.bytes);
        }
    }

    fn wr_scalar_fn(field: Box<dyn FieldProto>, data: ScalarValue, field_len: u32) {
        let mut buf = vec![];
        assert!(field.write(&mut buf, &data).is_ok());

        if field_len != 0 { assert_eq!(field_len, buf.len() as u32); }
        if field.wire_type() == WT_VARINT { assert_eq!(field_len, 0) }

        let mut counter = buf.len() as u32;
        let mut io_read = buf.as_slice();
        let mut read = PbReader::new(&mut io_read);
        if let Ok(data2) = field.read(&mut read, &mut counter, field_len) {
            assert_eq!(data, data2);
            assert_eq!(counter, 0);
        } else { panic!() }
    }

    #[test]
    fn write_and_read_integer_32_fields() {
        assert_eq!(Int32FieldProto::MIN, FixedInt32FieldProto::MIN);
        assert_eq!(Int32FieldProto::MAX, FixedInt32FieldProto::MAX);
        for value in [0, 0x55, 0x5555, -1, -999999999, Int32FieldProto::MIN, FixedInt32FieldProto::MIN] {
            wr_scalar_fn(Box::new(Int32FieldProto(CommonFieldProto::default())), ScalarValue::I32(value), 0);
            wr_scalar_fn(Box::new(FixedInt32FieldProto(CommonFieldProto::default())), ScalarValue::SF32(value), 4);
        }

        assert_eq!(UInt32FieldProto::MIN, FixedUInt32FieldProto::MIN);
        assert_eq!(UInt32FieldProto::MAX, FixedUInt32FieldProto::MAX);
        for value in [0, 0x55, 0x5555, UInt32FieldProto::MIN, FixedUInt32FieldProto::MIN] {
            wr_scalar_fn(Box::new(UInt32FieldProto(CommonFieldProto::default())), ScalarValue::U32(value), 0);
            wr_scalar_fn(Box::new(FixedUInt32FieldProto(CommonFieldProto::default())), ScalarValue::UF32(value), 4);
        }

        for value in [0, 0x55, 0x5555, -1, -999999999, SInt32FieldProto::MIN, SInt32FieldProto::MAX] {
            wr_scalar_fn(Box::new(SInt32FieldProto(CommonFieldProto::default())), ScalarValue::S32(value), 0);
        }
    }

    #[test]
    fn write_and_read_integer_64_fields() {
        assert_eq!(Int64FieldProto::MIN, FixedInt64FieldDefinition::MIN);
        assert_eq!(Int64FieldProto::MAX, FixedInt64FieldDefinition::MAX);
        for value in [0, 0x55, 0x5555, -1, -999999999999999999, i64::MAX, -i64::MAX, i64::MIN] {
            wr_scalar_fn(Box::new(Int64FieldProto(CommonFieldProto::default())), ScalarValue::I64(value), 0);
            wr_scalar_fn(Box::new(FixedInt64FieldDefinition(CommonFieldProto::default())), ScalarValue::SF64(value), 8);
        }
        assert_eq!(UInt64FieldProto::MIN, FixedUInt64FieldDefinition::MIN);
        assert_eq!(UInt64FieldProto::MAX, FixedUInt64FieldDefinition::MAX);
        for value in [0, 0x55, 0x5555, i64::MAX as u64, u64::MAX] {
            wr_scalar_fn(Box::new(UInt64FieldProto(CommonFieldProto::default())), ScalarValue::U64(value), 0);
            wr_scalar_fn(Box::new(FixedUInt64FieldDefinition(CommonFieldProto::default())), ScalarValue::UF64(value), 8);
        }
        for value in [0, 0x55, 0x5555, -1, -999999999999999999, SInt64FieldProto::MIN, -SInt64FieldProto::MAX] {
            wr_scalar_fn(Box::new(SInt64FieldProto(CommonFieldProto::default())), ScalarValue::S64(value), 0);
        }
    }
    #[test]
    fn write_and_read_float_fields() {
        for value in [0f32, 1f32, f32::MIN, f32::MAX] {
            wr_scalar_fn(Box::new(FloatFieldDefinition(CommonFieldProto::default())), ScalarValue::F32(value), 4);
        }

        for value in [0f64, -1f64, f64::MIN, f64::MAX] {
            wr_scalar_fn(Box::new(DoubleFieldDefinition(CommonFieldProto::default())), ScalarValue::F64(value), 8);
        }
    }
    #[test]
    fn write_and_read_bool_fields() {
        for value in [false, true] {
            wr_scalar_fn(Box::new(BoolFieldDefinition(CommonFieldProto::default())), ScalarValue::BOOL(value), 0);
        }
    }
    #[test]
    fn write_and_read_bytes_fields() {
        for value in [vec![], vec![0, 0, 0], vec![0xff; 300]] {
            let field_len = value.len() as u32;
            wr_scalar_fn(Box::new(BytesFieldDefinition(CommonFieldProto::default())), ScalarValue::BYTES(value), field_len as u32);
        }
    }
    #[test]
    fn write_and_read_string_fields() {
        for value in ["".to_string(), "abc".to_string(), "АВС".to_string(), String::new()] {
            let field_len = value.as_bytes().len() as u32;
            wr_scalar_fn(Box::new(StringFieldDefinition(CommonFieldProto::default())), ScalarValue::STR(value), field_len as u32);
        }
    }
}


#[cfg(test)]
mod read_message {
    use std::io;
    use std::io::Write;
    use crate::{App, TOP_LINE};
    use crate::proto::ProtoData;
    use crate::typedefs::PbReader;
    use crate::view::FieldOrder;
    use crate::wire::{FieldPos, FieldValue, MessageData};
    use crate::wire::ScalarValue::{I32, SF32, STR};

    fn all_scalar_proto() -> &'static str {
        r#"
syntax = "proto3";
message AllScalars {
int32 f_i32 = 10;
uint32 f_u32 = 11;
sint32 f_s32 = 12;
fixed32 f_fi32 = 13;
sfixed32 f_fs32 = 14;
int64 f_i64 = 20;
uint64 f_u64 = 21;
sint64 f_s64 = 22;
fixed64 f_fu64 = 23;
sfixed64 f_fi64 = 24;
float f_f32 = 30;
double f_f64 = 31;
bool f_bool = 40;
string f_str = 50;
bytes f_bytes = 60;
}
"#
    }

    #[test]
    fn scalars() {
        let binary_input =
            [0x50, 0x0B, 0x58, 0x0C, 0x60, 0x1A, 0x6D, 0x0E, 0x00, 0x00, 0x00, 0x75, 0x0F, 0x00, 0x00, 0x00, 0xA0, 0x01, 0x10, 0xA8, 0x01, 0x11, 0xB0, 0x01, 0x24, 0xB9, 0x01, 0x13, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC1, 0x01, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF5, 0x01, 0x00, 0x00, 0xA8, 0x41, 0xF9, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x36, 0x40, 0xC0, 0x02, 0x01, 0x92, 0x03, 0x06, 0x73, 0x74, 0x72, 0x69, 0x6E, 0x67, 0xE2, 0x03, 0x0A, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55];

        let proto = ProtoData::new(all_scalar_proto()).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();


        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg.clone(), &mut limit).unwrap();

        let expected = "message AllScalars {
  f_i32 = 11
  f_u32 = 12
  f_s32 = 13
  f_fi32 = 14
  f_fs32 = 15
  f_i64 = 16
  f_u64 = 17
  f_s64 = 18
  f_fu64 = 19
  f_fi64 = 20
  f_f32 = 21
  f_f64 = 22
  f_bool = true
  f_str = string
  f_bytes = 55 55 55 55 55 55 55 55 55 55
}
";
        assert_eq!(data.to_string(), expected);
        assert_eq!(data.get_field(&[(14, 0).into()]).unwrap().value, FieldValue::SCALAR(SF32(15)));
        assert!(data.get_field(&[(14, 1).into()]).is_none());
        assert!(data.get_field(&[(14, 0).into(), (1, 0).into()]).is_none());

        let mut output = Vec::new();
        data.write(&mut output, &proto, root_msg).unwrap();
        assert_eq!(output, binary_input);
    }

    #[test]
    fn scalars_max_values() { // all the numbers in maximal values
        let binary_input = [
            0x50, 0xFF, 0xFF, 0xFF, 0xFF, 0x07,                                     // int32#11 = 2147483647
            0x58, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F,                                     // uint32#12 = 4294967295
            // 0x60, 0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, // sint32#13 2147483647 wrong data from quick_protobus
            0x60, 0xFE, 0xFF, 0xFF, 0xFF, 0x0F, // sint32#13 2147483647 ok from protoscope
            0x6D, 0xFF, 0xFF, 0xFF, 0xFF,                                           // fixed32#14 = 4294967295
            0x75, 0xFF, 0xFF, 0xFF, 0x7F,                                           // sfixed32#15 = 2147483647
            0xA0, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F,       // int64#20 = 9223372036854775807
            0xA8, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, // uint64#21 = 18446744073709551615
            0xB0, 0x01, 0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, // sint64#22  =  9223372036854775807 = 2^63-1
            0xB9, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,             // fixed64#23 = 18446744073709551615 = 2^64-1
            0xC1, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F,             // sfixed64#24 = 9223372036854775807
            0xF5, 0x01, 0xFF, 0xFF, 0x7F, 0x7F,                                     // float#30 = 3.4028235e38
            0xF9, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xEF, 0x7F,             // double#31 = 1.7976931348623157e308
            0xC0, 0x02, 0x01]; // bool f_bool(#40) = true

        let proto = ProtoData::new(all_scalar_proto()).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg.clone(), &mut limit).unwrap();
        assert_eq!(binary_input.len(), data.len());

        let expected = "message AllScalars {
  f_i32 = 2147483647
  f_u32 = 4294967295
  f_s32 = 2147483647
  f_fi32 = 4294967295
  f_fs32 = 2147483647
  f_i64 = 9223372036854775807
  f_u64 = 18446744073709551615
  f_s64 = 9223372036854775807
  f_fu64 = 18446744073709551615
  f_fi64 = 9223372036854775807
  f_f32 = 3.4028235e38
  f_f64 = 1.7976931348623157e308
  f_bool = true
}
";
        assert_eq!(data.to_string(), expected);

        let mut output = Vec::new();
        data.write(&mut output, &proto, root_msg).unwrap();
        assert_eq!(output, binary_input);
    }


    #[test]
    fn scalars_min_values() { // all the numbers in minimal values
        let binary_input = [
            0x50, 0x80, 0x80, 0x80, 0x80, 0xF8, 0xFF, 0xFF, 0xFF, 0xFF, 0x01,       // int32#11
            0x60, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01,       // sint32#13
            0x75, 0x00, 0x00, 0x00, 0x80,                                           // sfixed32#15
            0xA0, 0x01, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01, // int64#20
            0xB0, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, // sint64#22
            0xC1, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80,             // sfixed64#24
            0xF5, 0x01, 0xFF, 0xFF, 0x7F, 0xFF,                                     // float#30
            0xF9, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xEF, 0xFF];            // double#31
        let proto = ProtoData::new(all_scalar_proto()).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        let expected = r#"message AllScalars {
  f_i32 = -2147483648
  f_s32 = -2147483647
  f_fs32 = -2147483648
  f_i64 = -9223372036854775808
  f_s64 = -9223372036854775807
  f_fi64 = -9223372036854775808
  f_f32 = -3.4028235e38
  f_f64 = -1.7976931348623157e308
}
"#;
        assert_eq!(data.to_string(), expected);

        // TODO test data is incorrect(?) (64 bits for 32 bits fields), need to compare with other pb implementations
        // let mut output = Vec::new();
        // msg.write(&mut output, &proto, &root_msg).unwrap();
        // assert_eq!(output, binary_input);
    }

    #[test]
    fn scalars_duplicated() {
        let binary_input = [0x50, 0x01, 0x50, 0x0B];
        let proto = ProtoData::new(all_scalar_proto()).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        // msg contains all the data read, but only the last value will be used
        assert_eq!(data.to_string(), "message AllScalars {\n  f_i32 = 1\n  f_i32 = 11\n}\n");
    }


    #[test]
    fn scalars_repeated_packed() {
        let binary_input = [0x32, 0x06, 0x03, 0x8e, 0x02, 0x9e, 0xa7, 0x05];

        let proto_str = r#"message Test5 {  repeated int32 f = 6;  }"#;

        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        let expected = "message Test5 {\n  f = 3\n  f = 270\n  f = 86942\n}\n";
        assert_eq!(data.to_string(), expected);
    }

    #[test]
    fn scalars_repeated() {
        let binary_input = [0x50, 0x01, 0x50, 0x0B];

        let proto_str = r#"message Test5 {  repeated int32 f = 10;  }"#;

        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();
        assert_eq!(binary_input.len(), data.len());

        let expected = "message Test5 {\n  f = 1\n  f = 11\n}\n";
        assert_eq!(data.to_string(), expected);

        assert_eq!(data.get_field(&[(10, 0).into()]).unwrap().value, FieldValue::SCALAR(I32(1)));
        assert_eq!(data.get_field(&[(10, 1).into()]).unwrap().value, FieldValue::SCALAR(I32(11)));
        assert!(data.get_field(&[(10, 2).into()]).is_none());
        assert!(data.get_field(&[(11, 0).into()]).is_none());
        assert!(data.get_field(&[(11, 2).into()]).is_none());
    }


    #[test]
    fn string_repeated() {
        let binary_input = [0x0A, 0x03, 0x61, 0x62, 0x63, 0x0A, 0x03, 0x41, 0x42, 0x43];
        let proto_str = "message StrRepeated {  repeated string s = 1; }";

        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        let expected = "message StrRepeated {\n  s = abc\n  s = ABC\n}\n";
        assert_eq!(data.to_string(), expected);
        assert_eq!(data.get_field(&[(1, 1).into()]).unwrap().value, FieldValue::SCALAR(STR("ABC".into())));
    }


    #[test]
    fn string_wrong_utf() {
        let binary_input = [0x0A, 0x01, 0xFF];
        let proto_str = r#"message StrTest {  repeated string s = 1; }"#;

        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();
        let mut read = PbReader::new(binary_input.as_slice());
        let h = MessageData::new(&mut read, &proto, root_msg, &mut limit);
        let msg = h.unwrap();

        let expected = "message StrTest {\n  s = wrong unicode data\n}\n";
        assert_eq!(msg.to_string(), expected);
    }

    #[test]
    fn empty_string() {
        let binary_input = [0x0A, 0x00];
        let proto_str = "message EmptyStr { string s = 1; }";

        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();
        assert_eq!(data.to_string(), "message EmptyStr {\n  s = \n}\n");
    }

    #[test]
    fn empty_message() {
        let binary_input = [0x12, 0x00];
        let proto_str = "message EmptyMsg { M2 m = 2; }\nmessage M2 { int32 f = 3; }";
        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();
        assert_eq!(data.to_string(), "message EmptyMsg {\n  m = message M2 {\n}\n\n}\n");
    }

    #[test]
    fn empty_input() {
        let binary_input = [];
        let proto_str = "message EmptyMsg { }";
        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();
        assert_eq!(data.to_string(), "message EmptyMsg {\n}\n");
    }

    #[test]
    fn bytes_repeated() {
        let binary_input = [0x0A, 0x03, 0x00, 0xFF, 0x00, 0x0A, 0x02, 0xFF, 0xEE];
        let proto_str = "message BytesRepeated { repeated bytes b = 1; }";

        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();
        let mut read = PbReader::new(binary_input.as_slice());
        let h = MessageData::new(&mut read, &proto, root_msg, &mut limit);
        let msg = h.unwrap();

        let expected = "message BytesRepeated {\n  b = 00 ff 00\n  b = ff ee\n}\n";

        assert_eq!(msg.to_string(), expected);
    }


    #[test]
    fn enums() { // all the numbers in minimal values
        let binary_input: [u8; 8] = [0x08, 0x02, 0x12, 0x04, 0x4A, 0x61, 0x63, 0x6B];
        let proto_str = "
enum PetType {
  CAT = 1;
  DOG = 2;
}
message Pet {
  PetType animal = 1;
  string name = 2;
}
";

        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg.clone(), &mut limit).unwrap();

        assert_eq!(data.to_string(), "message Pet {\n  animal = DOG\n  name = Jack\n}\n");

        let mut output = Vec::new();
        data.write(&mut output, &proto, root_msg).unwrap();
        assert_eq!(output, binary_input);
    }

    #[test]
    fn submessages() {
        let binary_input = [
            0x0A, 0x0A, // humans
            0x0A, 0x06, 0x4F, 0x6C, 0x69, 0x76, 0x65, 0x72, // Oliver
            0x10, 0x0A, // 10
            0x12, 0x08, 0x08, 0x02, 0x12, 0x04, 0x4A, 0x61, 0x63, 0x6B];

        let proto_str = "
enum PetType {
  CAT = 1;
  DOG = 2;
}
message Pet {
  PetType animal = 1;
  string name = 2;
}
message Human {
  string name = 1;
  int32 age = 2;
}
message House {
  Human humans = 1;
  Pet pets = 2;
}
";

        let proto = ProtoData::new(proto_str).unwrap();
        let root_msg = proto.root_message();
        assert!(proto.get_message_definition("Pet").is_some());
        assert!(proto.get_message_definition("Human").is_some());
        assert!(proto.get_message_definition("House").is_some());
        assert_eq!(root_msg.name, "House");


        let mut limit = binary_input.len() as u32;
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg.clone(), &mut limit).unwrap();

        assert_eq!(data.to_string(), "message House {\n  humans = message Human {\n  name = Oliver\n  age = 10\n}\n\n  pets = message Pet {\n  animal = DOG\n  name = Jack\n}\n\n}\n");
        assert_eq!(data.get_field(&[(1, 0).into(), (2, 0).into()]).unwrap().value, FieldValue::SCALAR(I32(10)));
        assert_eq!(data.get_field(&[(2, 0).into(), (2, 0).into()]).unwrap().value, FieldValue::SCALAR(STR("Jack".to_string())));
        assert!(data.get_field(&[(2, 0).into(), (3, 0).into()]).is_none());
        assert!(data.get_field(&[(2, 0).into(), (2, 3).into()]).is_none());

        let mut output = Vec::new();
        data.write(&mut output, &proto, root_msg).unwrap();
        assert_eq!(output, binary_input);
    }

    #[test]
    fn unknown_field() {
        let binary_input = [
            0x70, 0x01,                    // 14: 1
            0x75, 0x02, 0x03, 0x04, 0x05,  // 14: 6.2071626e-36i32  # 0x5040302i32
            0x72, 0x03, 0x65, 0x66, 0x67]; // 14: {"efg"}

        let proto_str = r#"message UnknownFieldTest { fixed32 unused_field = 555; }"#;

        //{
        //    let mut f = std::fs::File::create("binary_input.pb").unwrap();
        //    f.write_all(binary_input.as_slice()).unwrap();
        //}


        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg.clone(), &mut limit).unwrap();

        let expected = "message UnknownFieldTest {\n  ??? = 14.0: 01\n  ??? = 14.4: 02 03 04 05\n  ??? = 14.3: 65 66 67\n}\n";
        assert_eq!(data.to_string(), expected);
        assert!(data.get_field(&[(14, 0).into()]).is_some());
        assert!(data.get_field(&[(14, 2).into()]).is_some());
        assert!(data.get_field(&[(14, 3).into()]).is_none());
        assert!(data.get_field(&[(555, 0).into()]).is_none());

        let mut output = Vec::new();
        data.write(&mut output, &proto, root_msg).unwrap();
        assert_eq!(output, binary_input);
    }


    #[test]
    fn oneof() {
        let binary_input = [
            0xA5, 0x06, 0x00, 0x00, 0x80, 0x3F,  // float#100 = 1.0;
            0xAA, 0x06, 0x03, 0x61, 0x62, 0x63]; // string#101 = "abc";

        let proto_str = "message TestMessage { float length = 100; oneof test_oneof { string name = 101; int32 number = 102; }}";

        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();

        println!("{:?}", proto);

        assert_eq!(root_msg.fields.len(), 3);
        assert!(root_msg.fields[0].oneof_name().is_none());
        assert!(root_msg.fields[1].oneof_name().is_some());
        assert!(root_msg.fields[2].oneof_name().is_some());

        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();
        assert_eq!("message TestMessage {\n  length = 1\n  name = abc\n}\n", data.to_string());
        assert!(data.get_field(&[(100, 0).into()]).is_some());
        assert!(data.get_field(&[(101, 0).into()]).is_some());
        assert!(data.get_field(&[(102, 0).into()]).is_none());
    }

    #[test]
    fn oneof_duplicated() {
        let binary_input = [
            0xA5, 0x06, 0x00, 0x00, 0x80, 0x3F, // float#100 = 1.0;
            0xAA, 0x06, 0x03, 0x61, 0x62, 0x63, // string#101 = "abc";
            0xB0, 0x06, 0x64];                  // int32#102 = 100

        let proto_str = r#"message TestMessage { float length = 100; oneof test_oneof { string name = 101; int32 number = 102; }}"#;

        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();

        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();
        // msg contains all the data read, but only the last value will be used
        assert_eq!("message TestMessage {\n  length = 1\n  name = abc\n  number = 100\n}\n", data.to_string());
    }

    #[test]
    fn map() {
        let binary_input = [
            0x0A, 0x07, 0x08, 0x01, 0x12, 0x03, 0x66, 0x6F, 0x6F,
            0x0A, 0x07, 0x08, 0x02, 0x12, 0x03, 0x62, 0x61, 0x72
        ];

        let proto_str = "message TestMessage {  map<int32, string> dict = 1; }";

        let proto = ProtoData::new(proto_str).unwrap();
        assert!(proto.get_message_definition("TestMessage").is_some());
        // new message type created for the map field
        assert!(proto.get_message_definition("int32,string").is_some());

        let root_msg = proto.root_message();

        assert_eq!(root_msg.fields.len(), 1);

        assert!(root_msg.fields[0].is_message());

        let mut limit = binary_input.len() as u32;
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        let expected = "message TestMessage {\n  dict = message int32,string {\n  @1 = 1\n  @2 = foo\n}\n\n  dict = message int32,string {\n  @1 = 2\n  @2 = bar\n}\n\n}\n";
        assert_eq!(data.to_string(), expected);
        assert!(data.get_field(&[(1, 0).into(), (1, 0).into()]).is_some());
        assert!(data.get_field(&[(1, 0).into(), (2, 0).into()]).is_some());
    }

    #[test]
    fn add_field_private() {
        let binary_input = [];
        let proto_str = "message M1 { repeated int32 f1 = 1; }";

        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();

        let mut read = PbReader::new(binary_input.as_slice());
        let mut data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        assert!(data.add_field_private(2, 0).is_none());
        assert_eq!(data.to_string(), "message M1 {\n}\n");

        assert!(data.add_field_private(1, 0).is_some());
        assert_eq!(data.to_string(), "message M1 {\n  f1 = 0\n}\n");
        data.add_field_private(1, 1).unwrap().value = FieldValue::SCALAR(I32(1));
        assert_eq!(data.to_string(), "message M1 {\n  f1 = 0\n  f1 = 1\n}\n");
        data.add_field_private(1, 1).unwrap().value = FieldValue::SCALAR(I32(2));
        assert_eq!(data.to_string(), "message M1 {\n  f1 = 0\n  f1 = 2\n  f1 = 1\n}\n");
        data.add_field_private(1, 0).unwrap().value = FieldValue::SCALAR(I32(3));
        assert_eq!(data.to_string(), "message M1 {\n  f1 = 3\n  f1 = 0\n  f1 = 2\n  f1 = 1\n}\n");
    }

    #[test]
    fn remove_field_private() {
        let binary_input = [];
        let proto_str = "message M1 { repeated int32 f1 = 1; }";

        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();

        let mut read = PbReader::new(binary_input.as_slice());
        let mut data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();
        assert!(data.delete_field_private(1, 0).is_none());
        assert!(data.delete_field_private(2, 0).is_none());
        assert_eq!(data.to_string(), "message M1 {\n}\n");

        data.add_field_private(1, 0).unwrap().value = FieldValue::SCALAR(I32(0));
        data.add_field_private(1, 1).unwrap().value = FieldValue::SCALAR(I32(1));
        data.add_field_private(1, 2).unwrap().value = FieldValue::SCALAR(I32(2));
        data.add_field_private(1, 3).unwrap().value = FieldValue::SCALAR(I32(3));
        data.add_field_private(1, 4).unwrap().value = FieldValue::SCALAR(I32(4));
        assert_eq!(data.to_string(), "message M1 {\n  f1 = 0\n  f1 = 1\n  f1 = 2\n  f1 = 3\n  f1 = 4\n}\n");

        assert!(data.delete_field_private(1, 3).is_some());
        assert_eq!(data.to_string(), "message M1 {\n  f1 = 0\n  f1 = 1\n  f1 = 2\n  f1 = 4\n}\n");
        assert!(data.delete_field_private(1, 3).is_some());
        assert_eq!(data.to_string(), "message M1 {\n  f1 = 0\n  f1 = 1\n  f1 = 2\n}\n");
        assert!(data.delete_field_private(1, 3).is_none());
        assert!(data.delete_field_private(1, 0).is_some());
        assert!(data.delete_field_private(2, 0).is_none());
        assert_eq!(data.to_string(), "message M1 {\n  f1 = 1\n  f1 = 2\n}\n");
    }

    #[test]
    fn add_field() {
        let binary_input = [];
        let proto_str = "message M1 { int32 f1 = 1; M2 m2 = 2; }\nmessage M2 { int32 f2 = 3; }";

        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();

        let mut read = PbReader::new(binary_input.as_slice());
        let mut data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        assert!(data.add_field(&[(1, 0).into()]).is_some());
        assert_eq!(data.to_string(), "message M1 {\n  f1 = 0\n}\n");
        assert!(data.add_field(&[(2, 0).into()]).is_some());
        assert_eq!(data.to_string(), "message M1 {\n  f1 = 0\n  m2 = message M2 {\n}\n\n}\n");
        data.add_field(&[(2, 0).into(), (3, 0).into()]).unwrap().value = FieldValue::SCALAR(I32(10));
        assert_eq!(data.to_string(), "message M1 {\n  f1 = 0\n  m2 = message M2 {\n  f2 = 10\n}\n\n}\n");
    }

    #[test]
    fn delete_field() {
        let binary_input = [];
        let proto_str = "message M1 { int32 f1 = 1; M2 m2 = 2; }\nmessage M2 { int32 f2 = 3; }";

        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();

        let mut read = PbReader::new(binary_input.as_slice());
        let mut data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        assert!(data.add_field(&[(1, 0).into()]).is_some());
        assert!(data.add_field(&[(2, 0).into()]).is_some());
        data.add_field(&[(2, 0).into(), (3, 0).into()]).unwrap().value = FieldValue::SCALAR(I32(10));
        assert_eq!(data.to_string(), "message M1 {\n  f1 = 0\n  m2 = message M2 {\n  f2 = 10\n}\n\n}\n");

        data.delete_field(&[(1, 0).into()]);
        assert_eq!(data.to_string(), "message M1 {\n  m2 = message M2 {\n  f2 = 10\n}\n\n}\n");
        data.delete_field(&[(2, 0).into(), (3, 0).into()]);
        assert_eq!(data.to_string(), "message M1 {\n  m2 = message M2 {\n}\n\n}\n");
        assert!(data.delete_field(&[(2, 0).into()]).is_some());
        assert_eq!(data.to_string(), "message M1 {\n}\n");
    }

    #[test]
    fn sort_fields() {
        let binary_input = [
            0x10, 0x08,  // int32#b2 = 8
            0x18, 0x09,  // int32#a3 = 9
            0x18, 0x0A,  // int32#a3 = 10
            0x08, 0x0B,  // int32#c1 = 11
            0x18, 0x0C]; // int32#a3 = 12

        let proto_str = "message M1 { repeated int32 a3 = 3; int32 c1 = 1; int32 b2 = 2; int32 d4 = 4; }";
        let proto = ProtoData::new(proto_str).unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.root_message();
        let mut read = PbReader::new(binary_input.as_slice());
        let mut data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();
        assert_eq!(data.to_string(), "message M1 {\n  b2 = 8\n  a3 = 9\n  a3 = 10\n  c1 = 11\n  a3 = 12\n}\n");

        let sorted = data.get_sorted_fields(&FieldOrder::Wire);
        assert_eq!(sorted.len(), 4);
        assert_eq!(sorted[0], (FieldPos { id: 2, index: 0 }, 1));
        assert_eq!(sorted[1], (FieldPos { id: 3, index: 0 }, 2));
        assert_eq!(sorted[2], (FieldPos { id: 1, index: 0 }, 1));
        assert_eq!(sorted[3], (FieldPos { id: 3, index: 2 }, 1));

        let sorted = data.get_sorted_fields(&FieldOrder::ByName);
        assert_eq!(sorted.len(), 4);
        assert_eq!(sorted[0], (FieldPos { id: 3, index: 0 }, 3));
        assert_eq!(sorted[1], (FieldPos { id: 2, index: 0 }, 1));
        assert_eq!(sorted[2], (FieldPos { id: 1, index: 0 }, 1));
        assert_eq!(sorted[3], (FieldPos { id: 4, index: 0 }, 0));

        let sorted = data.get_sorted_fields(&FieldOrder::ById);
        assert_eq!(sorted.len(), 4);
        assert_eq!(sorted[0], (FieldPos { id: 1, index: 0 }, 1));
        assert_eq!(sorted[1], (FieldPos { id: 2, index: 0 }, 1));
        assert_eq!(sorted[2], (FieldPos { id: 3, index: 0 }, 3));
        assert_eq!(sorted[3], (FieldPos { id: 4, index: 0 }, 0));

        let sorted = data.get_sorted_fields(&FieldOrder::Proto);
        assert_eq!(sorted.len(), 4);
        assert_eq!(sorted[0], (FieldPos { id: 3, index: 0 }, 3));
        assert_eq!(sorted[1], (FieldPos { id: 1, index: 0 }, 1));
        assert_eq!(sorted[2], (FieldPos { id: 2, index: 0 }, 1));
        assert_eq!(sorted[3], (FieldPos { id: 4, index: 0 }, 0));


        //assert_eq!(data.get_next_field(FieldPos { id: 0, index: 0 }, FieldOrder::Binary).unwrap(), FieldPos { id: 3, index: 0 });


        //        assert_eq!(data.get_next_field(FieldPos{ id: 3, index: 0 }, FieldOrder::ByName).unwrap(), FieldPos{ id: 3, index: 1 });
        //        assert_eq!(data.get_next_field(FieldPos{ id: 3, index: 1 }, FieldOrder::ByName).unwrap(), FieldPos{ id: 2, index: 0 });
        //        assert_eq!(data.get_next_field(FieldPos{ id: 2, index: 0 }, FieldOrder::ByName).unwrap(), FieldPos{ id: 1, index: 0 });
        //        assert!(data.get_next_field(FieldPos{ id: 1, index: 0 }, FieldOrder::ByName).is_none());
        //
        //        assert_eq!(data.get_next_field(FieldPos{ id: 2, index: 0 }, FieldOrder::ById).unwrap(), FieldPos{ id: 3, index: 0 });
        //        assert!(data.get_next_field(FieldPos{ id: 3, index: 1 }, FieldOrder::ById).is_none());
        //
        //        assert_eq!(data.get_next_field(FieldPos{ id: 3, index: 0 }, FieldOrder::Proto).unwrap(), FieldPos{ id: 3, index: 1 });
        //        assert_eq!(data.get_next_field(FieldPos{ id: 3, index: 1 }, FieldOrder::Proto).unwrap(), FieldPos{ id: 1, index: 0 });
        //        assert_eq!(data.get_next_field(FieldPos{ id: 1, index: 0 }, FieldOrder::Proto).unwrap(), FieldPos{ id: 2, index: 0 });
        //        assert!(data.get_next_field(FieldPos{ id: 2, index: 0 }, FieldOrder::Proto).is_none());


        //pub fn get_next_field(&self, pos: FieldPos, order: FieldOrder) -> Option<FieldPos> {

    }


    #[test]
    fn bench_repeated_string() {
        let proto = ProtoData::new("message M { repeated string i1 = 1;  }").unwrap();
        let root_msg = proto.root_message();
        let mut read = PbReader::new([].as_slice());
        let mut data = MessageData::new(&mut read, &proto, root_msg, &mut 0).unwrap();

        // for now, without optimization app works with 1e4 lines,
        // the optimized version will be able to open at least 18000 messages * 100 lines per message (2e6)
        const COUNT: usize = 10000;
        for _ in 0..COUNT {
            data.add_field(&[(1, 0).into()]).unwrap();
        }

        assert_eq!(data.fields.len(), COUNT);

        const CONTENT_HEIGHT: u16 = 10;
        let mut app = App::for_tests(data, FieldOrder::Proto, 30, CONTENT_HEIGHT + TOP_LINE).unwrap();
        let screen = app.to_strings();

        assert_eq!(screen.len(), (CONTENT_HEIGHT as usize).min(COUNT));
        for line in screen {
            assert_eq!(line, " i1: ''               string* ");
        }
    }
}
