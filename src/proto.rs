use std::collections::HashSet;
use std::fmt::{Debug, Formatter};
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;
use pest::iterators::{Pairs};
use crate::typedefs::*;

use pest::Parser;
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "pb.pest"]
pub struct PBParser;


pub struct ProtoFile {
    path: PathBuf,
    pub content: String,
}

pub struct ProtoData {
    messages: Vec<MessageProtoPtr>,
    enums: Vec<EnumProtoPtr>,
    pub unknown_field: FieldProtoPtr, //UnknownFieldDefinition,
}

pub type FieldProtoPtr = Rc<dyn FieldProto>;
pub type MessageProtoPtr = Rc<MessageProto>;
pub type EnumProtoPtr = Rc<EnumProto>;

pub struct MessageProto {
    pub name: String,
    pub fields: Vec<FieldProtoPtr>,
    pub comment: String,
}

pub struct EnumProto {
    pub name: String,
    pub variants: Vec<(String, i32, String)>, // name, id, comment
    pub comment: String,
}

impl ProtoData {
    pub fn new(input: &str) -> io::Result<ProtoData> {
        match PBParser::parse(Rule::file, input) {
            Ok(rules_pairs) => {
                let proto_data = ProtoData::from_pairs(rules_pairs);
                Ok(proto_data)
            }
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e.to_string()))
        }
    }

    pub fn auto_detect_root_message(&self) -> Option<MessageProtoPtr> {

        // root message cannot be used as a field of another message (but can be himself field)
        let all_msg_names: HashSet<String> = self.messages.iter().map(|m| m.name.clone()).collect();

        // remove auto-created messages for map fields
        let all_msg_names = all_msg_names.into_iter().filter(|m| !m.contains(",")).collect();

        let mut sub_msg_names = vec![];
        for msg in &self.messages {
            for fld in &msg.fields {
//                if fld.is_message() { // unless the proto data finalized we do not know is it a message
                    if fld.typename() != msg.name {
                        sub_msg_names.push(fld.typename());
                    }
//                }
            }
        }

        let used_msg: HashSet<String> = sub_msg_names.into_iter().collect();

        let top_lvl_msg = &all_msg_names - &used_msg;

        if top_lvl_msg.len() == 1 {
            let top_msg_name = top_lvl_msg.iter().last().unwrap();
            let res = self.messages.iter().find(|&m| &m.name.as_str() == top_msg_name).unwrap();
            return Some(res.clone());
        }

        None
    }

    pub fn get_message_definition(&self, name: &str) -> Option<MessageProtoPtr> {
        if let Ok(index) = self.messages.binary_search_by(|m| m.name.as_str().cmp(name)) {
            Some(self.messages[index].clone())
        } else {
            None
        }
    }

    pub fn get_enum_definition(&self, name: &str) -> Option<&EnumProto> {
        if let Ok(index) = self.enums.binary_search_by(|m| m.name.as_str().cmp(name)) {
            Some(&self.enums[index])
        } else {
            None
        }
    }

    pub fn append(&mut self, mut other: ProtoData) {
        self.messages.append(&mut other.messages);
        self.enums.append(&mut other.enums);
    }

    fn add_message(pairs: Pairs<Rule>, comment: String) -> ProtoData {
        let mut it = pairs.into_iter(); // first get the message name
        let name_rule = it.next().unwrap();
        debug_assert_eq!(name_rule.as_rule(), Rule::name);
        let name = name_rule.as_span().as_str().to_string();
        let mut field_comment = String::new();

        let mut fields: Vec<Rc<dyn FieldProto>> = Vec::new(); // read message fields and other content
        let mut res = ProtoData { messages: vec![], enums: vec![], unknown_field: Rc::new(UnknownFieldDefinition::new()) };
        for pair in it {
            match pair.as_rule() {
                Rule::msg_field => {
                    fields.push(Self::field_from_pair(field_comment.clone(), pair.into_inner(), None));
                    field_comment.clear();
                }
                Rule::enum1 => {
                    res.enums.push(Self::add_enum(pair.into_inner(), field_comment.clone()));
                    field_comment.clear();
                }
                Rule::message => {
                    res.append(Self::add_message(pair.into_inner(), field_comment.clone()));
                    field_comment.clear();
                }
                Rule::one_of => {
                    let mut it = pair.into_inner().into_iter();
                    let name_rule = it.next().unwrap();
                    debug_assert_eq!(name_rule.as_rule(), Rule::name);
                    let oneof_name = Some(name_rule.as_span().as_str().to_string());

                    for pair in it {
                        match pair.as_rule() {
                            Rule::msg_field => {
                                fields.push(Self::field_from_pair(field_comment.clone(), pair.into_inner(), oneof_name.clone()));
                                field_comment.clear();
                            }
                            Rule::COMMENT => {
                                if !field_comment.is_empty() { field_comment += "\n"; }
                                field_comment += pair.as_span().as_str().trim_start_matches("//");
                            }
                            //Rule::option | Rule::EOI
                            _ => { panic!("Unknown oneof rule: {:?}", pair.as_rule()); }
                        }
                    }
                }
                Rule::COMMENT => {
                    if !field_comment.is_empty() { field_comment += "\n"; }
                    field_comment += pair.as_span().as_str().trim_start_matches("//");
                }
                Rule::mapname |
                Rule::option | Rule::EOI => {}
                _ => { panic!("Unknown message rule: {:?}", pair.as_rule()); }
            };
        }

        res.messages.push(Rc::new(MessageProto { name, fields, comment }));
        return res;
    }

    fn add_enum(pairs: Pairs<Rule>, comment: String) -> EnumProtoPtr {
        let mut variants = Vec::new();
        let mut field_comment = String::new();

        let mut it = pairs.into_iter();
        let name_rule = it.next().unwrap();
        debug_assert_eq!(name_rule.as_rule(), Rule::name);
        let name = name_rule.as_span().as_str().to_string();

        for pair in it {
            match pair.as_rule() {
                Rule::enum_field => {
                    let mut it = pair.into_inner();
                    let name = it.next().unwrap().as_str().to_string();
                    let value = it.next().unwrap().as_str().to_string();
                    variants.push((name, value.parse().unwrap(), field_comment.clone()));
                    field_comment.clear();
                    if let Some(r) = it.next() {
                        if r.as_rule() == Rule::COMMENT {
                            if !field_comment.is_empty() { field_comment += "\n"; }
                            field_comment += r.as_span().as_str().trim_start_matches("//");
                        }
                    }
                }
                Rule::option | Rule::EOI => {}
                _ => {
                    panic!("Unknown enum rule: {:?}", pair.as_rule());
                }
            };
        }

        Rc::new(EnumProto { name, variants, comment })
    }

    fn field_from_pair(comment: String, pairs: Pairs<Rule>, oneof_name: Option<String>) -> Rc<dyn FieldProto> {
        let mut name = String::new();
        let mut repeated = false;
        let mut type_name = String::new();
        let mut id = 0;
        //        let mut map_types : Option<(String, String)> = None;

        for pair in pairs {
            match pair.as_rule() {
                Rule::cardinality => {
                    repeated = match pair.as_span().as_str() {
                        "repeated" => true,
                        _ => false,
                    }
                }
                Rule::mapname => {
                    let mut it = pair.into_inner();
                    let key_type = it.next().unwrap().as_str().to_string();
                    let value_type = it.next().unwrap().as_str().to_string();
                    type_name = format!("{},{}", key_type, value_type);
                    //if repeated { warn!("map field ({}) cannot be repeated", name); }
                    repeated = true;
                }
                Rule::typename => {
                    type_name = pair.as_str().to_string();
                }
                Rule::name => {
                    name = pair.as_span().as_str().to_string();
                }
                Rule::integer => {
                    id = pair.as_span().as_str().parse().unwrap();
                }
                Rule::COMMENT | //=> { comments = comments + pair.as_span().as_str(); }
                Rule::option | Rule::EOI => {}
                _ => {
                    panic!("Unknown field rule: {:?}", pair.as_rule());
                }
            }
        };

        return CommonFieldProto::new_field(name, type_name, id, repeated, comment, oneof_name);
    }

    fn from_pairs(pairs: Pairs<Rule>) -> ProtoData {
        let mut res = ProtoData { messages: vec![], enums: vec![], unknown_field: Rc::new(UnknownFieldDefinition::new()) };
        let mut comments = String::new();
        for pair in pairs {
            for inner_pair in pair.into_inner() {
                match inner_pair.as_rule() {
                    //                    Rule::file => { return Self::from_pairs(inner_pair.into_inner()); }
                    Rule::message => {
                        res.append(Self::add_message(inner_pair.into_inner(), comments.clone()));
                        comments.clear();
                    }
                    Rule::enum1 => {
                        res.enums.push(Self::add_enum(inner_pair.into_inner(), comments.clone()));
                        comments.clear();
                    }
                    Rule::COMMENT => {
                        if !comments.is_empty() { comments += "\n"; }
                        comments += inner_pair.as_span().as_str().trim_start_matches("//");
                    }
                    Rule::option | Rule::EOI => {}
                    _ => {
                        panic!("Unknown rule: {:?}", inner_pair.as_rule());
                    }
                };
            }
        }
        //        res.create_map_messages();
        //        res.messages.sort_by(|a, b| a.name.cmp(&b.name));
        //        res.enums.sort_by(|a, b| a.name.cmp(&b.name));
        //        res.link_user_types();
        res
    }

    fn create_map_messages(&mut self) {
        let mut map_names = vec![]; // collect maps fields from all messages
        for msg in &self.messages {
            for field in &msg.fields {
                if field.typename().contains(',') {
                    map_names.push(field.typename());
                }
            }
        }
        // remove duplicated map types
        let map_names_hashset: HashSet<String> = map_names.into_iter().collect();

        // add new messages types for each found map type
        for name in map_names_hashset {
            let mut fields = vec![];
            let mut id = 1;
            for field_type in name.split(",") {
                fields.push(CommonFieldProto::new_field(format!("@{}", id),
                                                        field_type.to_string(), id,
                                                        false,
                                                        String::new(), None));
                id += 1;
            }
            self.messages.push(Rc::new(MessageProto { name, fields, comment: String::new() }));
        }
    }

    //    fn link_user_types(&mut self) {
    //        for msg in &self.messages {
    //            for field in &msg.fields {
    //                field.link_user_types(&self.enums, &self.messages);
    //            }
    //        }
    //    }


    pub fn finalize(mut self) -> io::Result<ProtoData> {
        self.create_map_messages();
        self.messages.sort_by(|a, b| a.name.cmp(&b.name));
        self.enums.sort_by(|a, b| a.name.cmp(&b.name));
        //self.link_user_types();

        for msg in &self.messages {
            for field in &msg.fields {
                field.link_user_types(&self.enums, &self.messages);
            }
        }

        // self.messages.sort_by(|a, b| a.name.cmp(&b.name));
        // self.enums.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(self)
    }
}

impl MessageProto {
    pub fn get_field(&self, number: i32) -> Option<FieldProtoPtr> {
        if let Some(fd) = self.fields.iter().find(|m| m.id() == number) {
            return Some(fd.clone());
        }
        None
    }
}

impl Debug for ProtoData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for msg in &self.messages {
            write!(f, "{:?}", msg)?;
        }
        for enm in &self.enums {
            write!(f, "{:?}", enm)?;
        }
        Ok(())
    }
}
impl Debug for MessageProto {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "message {} {{", self.name)?;

        let mut oneof = String::new();
        //let mut oneof3: Option<String> = None;

        for field in &self.fields {
            let mut oneof2 = String::new();
            if let Some(ofn) = field.oneof_name() {
                oneof2 = ofn.clone();
            }


            let new_oneof = field.oneof_name().clone();

            //if oneof3 != new_oneof {
            //    if new_oneof.is_some() {
            //        writeln!(f, "  oneof {} {{", oneof3.unwrap())?;
            //    }
            //    oneof3 = new_oneof;
            //}

            if oneof != oneof2 {
                oneof = oneof2.clone();
                writeln!(f, "  oneof {} {{", oneof)?;
            }

            if !oneof.is_empty() { write!(f, "  ")?; }

            write!(f, "  {:?}", field)?;
        }
        if !oneof.is_empty() {
            writeln!(f, "  }}")?;
        }

        writeln!(f, "}}")
    }
}

impl Debug for EnumProto {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "enum {} {{", self.name)?;
        for variant in &self.variants {
            writeln!(f, "  {} = {};", variant.0, variant.1)?;
        }
        writeln!(f, "}}")
    }
}


impl ProtoFile {
    fn new(path: PathBuf) -> ProtoFile {
        let content = std::fs::read_to_string(&path).unwrap();
        ProtoFile { path, content }
    }

    // https://protobuf.dev/programming-guides/proto3/#importing
    pub fn new_with_imports(name: PathBuf, proto_path: Vec<PathBuf>) -> Vec<ProtoFile> {
        let mut all_files = vec![];
        let mut files: Vec<ProtoFile> = vec![ProtoFile::new(name)];
        loop {
            // add children, all for the top level and only public children for others
            let new_files: Vec<ProtoFile> = files.iter().
                flat_map(|file| file.read_imports(&proto_path, all_files.is_empty())).
                collect();
            all_files.append(&mut files);
            if new_files.is_empty() { return all_files; }

            // remove files already in the list (circular dependency)
            files = new_files.into_iter().filter(|new| {
                all_files.iter().find(|&old| old.path == new.path).is_none()
            }).collect();
        }
    }

    fn extract_imports(&self) -> Vec<(String, bool)> { // (file_name, is_public)
        let mut res = vec![];
        for line in self.content.lines() {
            let lise_string = line.to_string();
            let s = lise_string.trim();
            if s.starts_with("import") { // import "file_path.proto";
                let s = s.trim_end_matches(';');
                let s = s.trim_start_matches("import");
                let s = s.trim();
                let s1 = s.trim_start_matches("public");
                let is_public = s1.len() != s.len();
                let s = s1.trim();
                let s = s.trim_matches('\"');
                res.push((s.to_string(), is_public));
            }
        }
        res
    }

    // search file by name in all possible locations
    fn resolve_path(&self, name: &str, proto_path: &Vec<PathBuf>) -> Option<PathBuf> {
        if let Ok(name) = PathBuf::from_str(name) {

            // as written in the import directive
            if let Ok(absolute) = std::path::absolute(&name) {
                if absolute.is_file() {
                    return Some(absolute);
                }
            }
            if name.is_relative() {

                // relative to current proto file
                if let Some(parent_path) = self.path.parent() {
                    let file_path = parent_path.join(&name);
                    if file_path.is_file() {
                        return Some(file_path);
                    }
                }

                // search in the provided list of directories
                for dir in proto_path {
                    let file_path = dir.join(&name);
                    if file_path.is_file() {
                        return Some(file_path);
                    }
                }
            }
        }
        eprintln!("Imported file {name} not found");
        None
    }

    fn read_imports(&self, proto_path: &Vec<PathBuf>, all: bool) -> Vec<ProtoFile> {
        let mut res = vec![];
        for import_name in self.extract_imports().into_iter() {
            if all || import_name.1 {
                if let Some(path) = self.resolve_path(&import_name.0, &proto_path) {
                    let new = Self::new(path);
                    res.push(new);
                }
            }
        }
        res
    }
}


/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/
/**************************************************************************************************/


#[cfg(test)]
mod parsing {
    use super::*;

    const TEST_DATA_DIR: &'static str = r"C:\V\prj\rust\p18089\test-data-maker\data\";

    #[test]
    fn conformance() {
        for path in [
            // https://github.com/protocolbuffers/protobuf/blob/main/conformance/conformance.proto
            "conformance.proto",
            // https://github.com/protocolbuffers/protobuf/blob/main/src/google/protobuf/test_messages_proto3.proto
            "test_messages_proto3.proto",
            "addressbook.proto",
        ] {
            let path = TEST_DATA_DIR.to_string() + path;
            assert!(ProtoData::new(std::fs::read_to_string(path).unwrap().as_str()).unwrap().finalize().is_ok());
        }
    }

    #[test]
    fn nested() {
        let proto_str = r#"message TestMessage {

  message NestedMessage {
    int32 a = 1;
  }

  enum NestedEnum {
    FOO = 0;
    BAR = 1;
    NEG = -1;
  }
}"#;
        let proto = ProtoData::new(proto_str).unwrap().finalize().unwrap();

        assert_eq!(proto.messages.len(), 2);
        assert_eq!(proto.enums.len(), 1);
        assert!(proto.get_message_definition("TestMessage").is_some());
        assert!(proto.get_message_definition("NestedMessage").is_some());
        assert!(proto.get_enum_definition("NestedEnum").is_some());
    }


    #[test]
    fn duplicated_maps() {
        let proto_str = r#"message TestMessage {
          map<int32, string> f1 = 1;
          map<int32, string> f2 = 2;
          map<int32, fixed32> f2 = 3;
        }"#;
        let proto = ProtoData::new(proto_str).unwrap().finalize().unwrap();
        assert_eq!(proto.messages.len(), 3);
        assert!(proto.get_message_definition("TestMessage").is_some());
        assert!(proto.get_message_definition("int32,string").is_some());
        assert!(proto.get_message_definition("int32,fixed32").is_some());
    }


    #[test]
    fn comments() {
        let proto_str = r#"
//comment 1
message TestMessage {
  //comment 2
  int32 a = 1;
}
//multiline
//comment 3
enum NestedEnum {
    FOO = 0;
    //comment 4
    BAR = 1;
}
"#;
        let proto = ProtoData::new(proto_str).unwrap().finalize().unwrap();
        assert_eq!(proto.messages.len(), 1);
        let msg = proto.auto_detect_root_message().unwrap();
        assert_eq!(msg.comment, "comment 1");
        assert_eq!(msg.fields.len(), 1);
        assert_eq!(msg.fields[0].comment(), "comment 2");

        let enum0 = &proto.enums[0];
        assert_eq!(enum0.comment, "multiline\ncomment 3");
        assert_eq!(enum0.variants[1].2, "comment 4");
    }


    #[test]
    fn auto_detect_root_message() {
        {
            let proto = ProtoData::new("message M1 { M2 m = 2; }\nmessage M2 { }").unwrap();
            assert_eq!(proto.auto_detect_root_message().unwrap().name, "M1");
        }
        {
            let proto = ProtoData::new("message M1 { M2 m = 2; }\nmessage M2 { }\nmessage M3 { }").unwrap();
            let root_msg = proto.auto_detect_root_message().is_none();
        }
        {
            let proto = ProtoData::new("message M1 { M2 m = 2; }\nmessage M2 { }\nmessage M3 { M1 m = 1; }").unwrap();
            assert_eq!(proto.auto_detect_root_message().unwrap().name, "M3");
        }
        {
            let proto = ProtoData::new("message M1 { M2 m = 2; }\nmessage M2 { M1 m = 1; }").unwrap();
            assert!(proto.auto_detect_root_message().is_none());
        }
        {
            let proto = ProtoData::new("").unwrap();
            assert!(proto.auto_detect_root_message().is_none());
        }
    }


    #[test]
    fn import_files() {
        let proto_file = ProtoFile::new((TEST_DATA_DIR.to_string() + "test_messages_proto3.proto").into());
        assert_eq!(proto_file.extract_imports(), [
            ("google/protobuf/any.proto".to_string(), false),
            ("google/protobuf/duration.proto".to_string(), false),
            ("google/protobuf/field_mask.proto".to_string(), false),
            ("google/protobuf/struct.proto".to_string(), false),
            ("google/protobuf/timestamp.proto".to_string(), false),
            ("google/protobuf/wrappers.proto".to_string(), false),
        ]);
    }

    #[test]
    fn import_files_public() {
        let proto_file = ProtoFile::new((TEST_DATA_DIR.to_string() + "import_tests/1.proto").into());
        assert_eq!(proto_file.extract_imports(), [
            ("2.proto".to_string(), false),
            ("3.proto".to_string(), true),
            ("dir/4.proto".to_string(), false),
        ]);
    }

    #[test]
    fn import_files_1() { // 1.proto -> import 3 files
        let files = ProtoFile::new_with_imports((TEST_DATA_DIR.to_string() + "import_tests/1.proto").into(), vec![]);
        assert_eq!(files.len(), 4);
    }

    #[test]
    fn import_files_5() { // 5.proto -> 6.proto (7.proto not imported because it is not public)
        let files = ProtoFile::new_with_imports((TEST_DATA_DIR.to_string() + "import_tests/5.proto").into(), vec![]);
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn import_files_8() { // 8.proto -> 9.proto -> 7.proto
        let files = ProtoFile::new_with_imports((TEST_DATA_DIR.to_string() + "import_tests/8.proto").into(), vec![]);
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn import_files_10() { // 10.proto -> dir/11.proto -> dir/4.proto (file in the same dir as parent)
        let files = ProtoFile::new_with_imports((TEST_DATA_DIR.to_string() + "import_tests/10.proto").into(), vec![]);
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn import_files_12() { // 12.proto -> dir/4.proto (file found in the proto_path)
        let files = ProtoFile::new_with_imports((TEST_DATA_DIR.to_string() + "import_tests/12.proto").into(),
                                                vec![(TEST_DATA_DIR.to_string() + "import_tests/dir/").into()]);
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn import_files_13() { // 13.proto -> 13.proto ...
        let files = ProtoFile::new_with_imports((TEST_DATA_DIR.to_string() + "import_tests/13.proto").into(), vec![]);
        assert_eq!(files.len(), 1);
    }
}
