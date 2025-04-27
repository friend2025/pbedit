use std::string::String;
use std::cmp::{Ordering, PartialEq};
use std::collections::{HashMap, HashSet};
use std::fmt::{Debug, Formatter};
use std::{io, iter, mem};
use crossterm::event::{KeyEvent};
use crossterm::style;
use crossterm::style::Color;
use crate::proto::FieldProtoPtr;
use crate::Selection;
use crate::trz::{Change, ChangeType};
use crate::wire::{FieldPath, FieldValue, MessageData, ScalarValue};
use crate::wire::ScalarValue::{BYTES, STR};

pub(crate) const MARGIN_RIGHT: u16 = 1;
pub(crate) const MARGIN_LEFT: u16 = 1;


pub enum UserCommand
{
    Refresh,
    // keys up,down,pageup,pagedown, or mouse wheel
    // true - move up
    ScrollVertically(usize, bool),
    ScrollSibling(i8),
    ScrollToBottom,
    ScrollHorizontally(i8),
    Home,
    End,
    // hotkey: 'C' comments visibility (enum CommentVisibility)
    CommentsVisibility,
    // hotkey: 'B'
    // show/hide binary data (read only)
    // the data shown only in tree mode, before main data line,
    // binary content (tag and length) explained, for example:
    // 002F8 : 0A 2:LEN 8C 35 2248 00 00 00 00 00 0F
    BinaryVisibility,
    // hotkey: 'P'
    // show/hide tyPe (by default shown only in a few cases)
    DataTypeVisibility,
    // hotkey: '→', '←' on collapsed field name
    CollapsedToggle,
    // hotkey: 'T'
    // tree / table mode switch) (vert/horiz auto select by content)
    TableTreeToggle,
    // hotkey: 'V'
    // switch vertical or regular table, in table view
    TableVariant,
    // hotkeys: '>','<'
    // increase or decrease column count
    ChangeColumnCount(i8),
    // hotkey: 'F4'
    // field Order in table or message (enum FieldOrder)
    ChangeFieldOrder(FieldOrder),
    // hotkey: 'Ctrl+←', 'Ctrl+→'
    // shift repeated scalar or table column
    MoveField,
    // hotkeys: Del/Ins
    // delete or create a repeated scalar or message
    DeleteData,
    InsertData,
    // hotkeys: 'E' ,'I'
    // supported file format depend on data types, show in UI
    // and detected by entered file name (txt, bin, pb, csv, tsv, json)
    //ExportData,
    //ImportData,
    // hotkey 'S', when selected column name of a repeated message in table mode
    // sort table by this column по (a...z|z...a|as read from file)
    SortDataView,
    // not a command, just key pressed
    KeyPress(KeyEvent),
}

pub enum CommandResult {
    None,
    Redraw,
    ChangeData(Change),
    ShowMenu(Vec<String>),
    ShowMessage(String),
    ShowError(String),
    StartEdit(FieldPath, u16, u16),
}

#[derive(Debug, PartialEq)]
pub enum LayoutType {
    Scalar,
    Bytes,
    Str,
    Message,
    Table,
    Collapsed,
}

pub struct LayoutParams {
    // how many lines occupies this layout on the screen
    pub height: usize,
    pub path: FieldPath,
    pub layout: Option<Box<dyn ViewLayout>>,
    pub sibling_index: usize, // index of this field in the parent
    pub sibling_count: usize, // how many siblings has this field's the parent
}

pub struct Layouts { // rename Document
    pub width: u16,
    pub height: u16,
    pub scroll: usize,
    pub items: Vec<LayoutParams>,
    pub file_name: String,
    pub indents: Vec<u16>,
}

// does not store data, only params how to display it
// in next versions: multiple cursor, selection and highlight (found item, etc.)
pub trait ViewLayout {
    fn layout_type(&self) -> LayoutType;
    fn amount(&self) -> usize; // TODO? move to LayoutParams
    // return how many screen lines take this layout
    fn calc_sizes(&mut self, root: &MessageData, path: &FieldPath, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) -> usize;
    // TODO first_line: usize, line_count: u16
    fn get_screen(&self, root: &MessageData, path: &FieldPath, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines;
    fn on_command(&mut self, root: &MessageData, path: &FieldPath, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, cursor_x: &mut u16, cursor_pos: &mut usize) -> CommandResult;
    // get ids of children fields already shown in this layout
    fn get_consumed_fields(&self, root: &MessageData, path: &FieldPath, config: &LayoutConfig) -> HashSet<i32> { HashSet::new() }
    fn get_status_string(&self, cursor_x: u16, cursor_y: usize) -> String { String::new() }
}


// bool, enum, integral, or real value: single, none or repeated
// there are special layouts for text and hex field types
pub struct ScalarLayout {
    amount: usize, // how many data with the same id, started from the provided index
    line_lens: Vec<usize>, // how many scalar values of each line on the screen
}
pub struct StringLayout {
    has_value: bool,
    visible_lines_count: usize, // TODO
}
pub struct BytesLayout {
    has_value: bool,
    bytes_per_line: u16,
    data_size: usize,
    //visible_lines_count: usize, // TODO
}
pub struct MessageLayout { // with columns or title only
    amount: usize,
    scroll: usize, // first visible column index
}
pub struct TableLayout { // for repeated messages
    amount: usize,
    vertical: bool,
    scroll: (usize, usize), // column and row indexes of top-left visible cell
}

pub struct CollapsedLayout {
    display_size: usize,
}

pub enum CommentVisibility {
    Hidden,
    Multiline, // before data, possible multiline
    Inline,    // in the same line, after data and type, only one line of comment
}
#[derive(PartialEq, Debug)]
pub enum FieldOrder {
    Proto,  // as in proto file (default)
    Wire,   // as the data read from the file, repeated may be in several groups
    ByName, // alphabetically by the name of the field
    ById,   // by numerical field id
}

pub struct LayoutConfig {
    pub show_comments: CommentVisibility,
    pub show_binary: bool,
    pub show_data_types: bool,
    pub field_order: FieldOrder,
    pub messages: HashMap<String, MessageLayoutConfig>,
    pub hex: bool,
}

// How to show a message or table of a certain type
pub struct MessageLayoutConfig {
    columns: Vec<i32>,
    columns_width: Vec<u16>,
}

#[derive(PartialEq, Debug, Copy, Clone)]
#[repr(u8)]
pub enum TextStyle {
    Comment,
    Binary,
    Filename,
    FieldName, // column header
    SelectedFieldName,
    FieldIndex, // left column
    SelectedFieldIndex,
    Value, // data content
    SelectedValue,
    DefaultValue,
    DataSize, // size of collapsed field
    Typename, // name of scalar type
    SelectedTypename, // for oneof
    Divider,
    Bookmark,
    TopLine, // top line with different status information
    Unknown,
}

pub struct ScreenLine(pub Vec<(char, TextStyle)>);

impl Default for LayoutConfig {
    fn default() -> Self {
        LayoutConfig {
            show_comments: CommentVisibility::Hidden,
            show_binary: false,
            show_data_types: false,
            field_order: FieldOrder::Proto,
            messages: HashMap::new(),
            hex: false,
        }
    }
}

impl CommentVisibility {
    pub fn next(&self) -> CommentVisibility {
        match self {
            CommentVisibility::Hidden => CommentVisibility::Inline,
            CommentVisibility::Inline => CommentVisibility::Multiline,
            CommentVisibility::Multiline => CommentVisibility::Hidden,
        }
    }
}
impl FieldOrder {
    pub fn next(&self) -> FieldOrder {
        match self {
            FieldOrder::Proto => FieldOrder::Wire,
            FieldOrder::Wire => FieldOrder::ByName,
            FieldOrder::ByName => FieldOrder::ById,
            FieldOrder::ById => FieldOrder::Proto,
        }
    }
    pub fn prev(&self) -> FieldOrder {
        match self {
            FieldOrder::Proto => FieldOrder::ById,
            FieldOrder::Wire => FieldOrder::Proto,
            FieldOrder::ByName => FieldOrder::Wire,
            FieldOrder::ById => FieldOrder::ByName,
        }
    }
    pub fn first_letter(&self) -> char {
        match self {
            FieldOrder::Proto => 'P',
            FieldOrder::Wire => 'W',
            FieldOrder::ByName => 'N',
            FieldOrder::ById => 'I',
        }
    }
}

impl ScreenLine {
    pub fn new(width: u16) -> ScreenLine { ScreenLine(Vec::with_capacity(width as usize)) }

    pub fn add_string(&mut self, text: String, style: TextStyle) {
        let mut new_item = text.chars().map(|c| (c, style)).collect::<Vec<(char, TextStyle)>>();
        self.0.append(&mut new_item);
    }

    pub fn add_field_name(&mut self, text: String, indent: u16, cursor: &Option<(u16, usize)>) {
        self.add_first_column_item([TextStyle::FieldName, TextStyle::SelectedFieldName], text, indent, cursor, 0);
    }
    pub fn add_value_address(&mut self, text: String, indent: u16, cursor: &Option<(u16, usize)>, cursor_pos: usize) {
        self.add_first_column_item([TextStyle::FieldIndex, TextStyle::SelectedFieldIndex], text, indent, cursor, cursor_pos);
    }
    fn add_first_column_item(&mut self, styles: [TextStyle; 2], text: String, indent: u16, cursor: &Option<(u16, usize)>, cursor_pos: usize) {
        let mut selected = false;
        if let Some((x, pos)) = cursor {
            selected = *x == 0 && *pos == cursor_pos;
        }
        if selected {
            for _ in 1..indent as usize - text.len() {
                self.0.push((' ', styles[0]));
            }
            self.0.push((' ', styles[1]));
            self.add_string(text, styles[1]);
            self.0.push((':', styles[1]));
        } else {
            let width = indent as usize;
            self.add_string(format!("{text:>width$}"), styles[0]);
            self.0.push((':', TextStyle::Divider));
        }
    }
    pub fn add_field_size(&mut self, value: usize, screen_width: u16) {
        //self.data_size = Some(value);
        //let width = screen_width - self.0.len() as u16 - MARGIN_RIGHT;
        let s = format!(" ... {}", value);
        self.add_string(s, TextStyle::DataSize);
    }
    pub fn add_typename(&mut self, field_def: FieldProtoPtr, screen_width: u16, empty: bool) {
        let mut text = field_def.typename();
        if field_def.repeated() { text = text + "*" }
        if empty { text = "-".to_string() + text.as_str() }
        let max_allowed_len = (screen_width - MARGIN_RIGHT) as usize - text.len();
        if self.0.len() > max_allowed_len {
            self.0.truncate(max_allowed_len);
        }
        let width = (screen_width - MARGIN_RIGHT) as usize - self.0.len();
        self.add_string(format!("{text:>width$}"), TextStyle::Typename);
        for _ in 0..MARGIN_RIGHT { self.0.push((' ', TextStyle::Typename)); }
    }

    pub fn fix_length(&mut self, len: u16) {
        let len = len as usize;
        match self.0.len().cmp(&len) {
            Ordering::Less => {
                let mut spaces = iter::repeat_n((' ', TextStyle::Divider), len - self.0.len()).collect();
                self.0.append(&mut spaces);
            }
            Ordering::Greater => {
                self.0.truncate(len);
            }
            Ordering::Equal => {}
        }
    }
}
pub struct ScreenLines(pub Vec<ScreenLine>);
impl ScreenLines {
    pub fn new() -> ScreenLines { ScreenLines(Vec::with_capacity(100)) }
    pub fn append(&mut self, other: &mut ScreenLines) { self.0.append(&mut other.0); }
}

pub struct IndentsCalc {
    level_indents: Vec<u16>,
}

impl IndentsCalc {
    const NEXT_LEVEL_INDENT: u16 = 2;

    pub fn new() -> IndentsCalc {
        IndentsCalc {
            level_indents: Vec::with_capacity(8)
        }
    }

    pub fn new_for_update(indents: Vec<u16>) -> IndentsCalc {
        IndentsCalc {
            level_indents: indents
        }
    }

    pub fn add(&mut self, first_column_width: usize /* TODO u16*/, mut level: usize) -> u16 {
        debug_assert!(level >= 1);
        level -= 1;
        while self.level_indents.len() <= level {
            let new_indent = self.level_indents.last().unwrap_or(&0);
            self.level_indents.push(Self::NEXT_LEVEL_INDENT + *new_indent);
        }

        let new_width = MARGIN_LEFT + first_column_width as u16;
        if self.level_indents[level] < new_width {
            self.level_indents[level] = new_width;
            for i in level + 1..self.level_indents.len() {
                self.level_indents[i] = self.level_indents[i - 1] + Self::NEXT_LEVEL_INDENT;
            }
        }
        self.level_indents[level]
    }
}

impl Into<Vec<u16>> for IndentsCalc {
    fn into(self) -> Vec<u16> {
        self.level_indents
    }
}


impl ScalarLayout {
    const MARGIN: u16 = MARGIN_LEFT + MARGIN_RIGHT;

    fn new(amount: usize) -> Self {
        ScalarLayout { amount, line_lens: vec![] }
    }
    fn add_scalar_value(line: &mut ScreenLine, value: &ScalarValue, def: &FieldProtoPtr, config: &LayoutConfig, selected: bool) {
        line.0.push((' ', TextStyle::Divider));
        let style = if selected { TextStyle::SelectedValue } else { TextStyle::Value };
        line.add_string(Self::scalar_to_string(value, def, config), style);
    }
    fn scalar_to_string(value: &ScalarValue, def: &FieldProtoPtr, config: &LayoutConfig) -> String {
        if let ScalarValue::ENUM(value) = value {
            if let Some(text) = def.get_enum_name_by_index(*value) {
                text.to_string()
            } else {
                format!("?{}", *value)
            }
        } else {
            //            if config.hex {
            //                format!("{:X}", value) // TODO
            //            } else {
            format!("{}", value)
            //            }

        }
    }

    fn get_line_lens(&self, full_width: u16, indent: u16, def: &FieldProtoPtr, msg: &MessageData, path: &FieldPath, config: &LayoutConfig) -> Vec<usize> {
        let mut avail_width = (full_width - indent - Self::MARGIN) as usize;
        if def.repeated() { avail_width -= 1 }
        avail_width -= def.typename().len();

        debug_assert!(self.amount > 0);
        let mut cur_len = 0;
        //let mut line_count = 1;

        let mut starts = vec![];
        let mut prv_line_end = 0;

        if let Some(last_pos) = path.0.last() {
            for index in last_pos.index..last_pos.index + self.amount {
                if let Some(field) = msg.get_field(&([(last_pos.id, index).into()])) {
                    if let FieldValue::SCALAR(value) = &field.value {
                        let str_value = Self::scalar_to_string(value, def, config);
                        let len = str_value.len();
                        cur_len += len + 1;
                        if cur_len >= avail_width {
                            cur_len = len + 1;
                            //line_count += 1;
                            starts.push(index - prv_line_end);
                            prv_line_end = index;
                            avail_width = (full_width - indent - Self::MARGIN) as usize;
                        }
                    }
                }
            }
            let last_line_len = last_pos.index + self.amount - prv_line_end;
            if last_line_len > 0 { starts.push(last_line_len) }
        }

        starts //line_count
    }

    fn data_index_at_cursor(&self, cursor_x: u16, mut cursor_y: usize) -> usize {
        if cursor_x == 0 { return usize::MAX; } // selected field name, no data
        let at_line_start: usize = self.line_lens.iter().take(cursor_y).map(|i| *i as usize).sum();
        at_line_start + cursor_x as usize - 1
    }

    fn cursor_at_data_index(&self, index: usize) -> (u16, usize) {
        let mut sum = 0;
        for line_index in 0..self.line_lens.len() {
            let line_len = self.line_lens[line_index];
            if sum + line_len >= index {
                return ((index - sum + 1) as u16, line_index);
            }
            sum += line_len;
        }
        (0, self.line_lens.len())
    }
}
impl ViewLayout for ScalarLayout {
    fn layout_type(&self) -> LayoutType { LayoutType::Scalar }
    fn amount(&self) -> usize { self.amount }
    fn get_status_string(&self, cursor_x: u16, cursor_y: usize) -> String {
        format!("/{}", self.amount)
    }
    fn calc_sizes(&mut self, root: &MessageData, path: &FieldPath, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) -> usize {
        if let Some(field_proto) = root.get_field_definition(path) {
            let field_name_length = field_proto.name().len();
            let level = path.0.len();
            let indent = negotiator.add(field_name_length, level);

            let mut line_count = 1;
            if self.amount > 0 {
                let mut p = path.0.clone();
                p.pop();
                if let Some(msg) = root.get_submessage(&p) {
                    self.line_lens = self.get_line_lens(width, indent, &field_proto, msg, path, config);
                    line_count = self.line_lens.len();
                }
            }
            return line_count.max(1); // if no data, default value will be shown
        }
        panic!("cannot layout")
    }

    // TODO    fn index_by_coordinates(&self, root: &MessageData, path: &FieldPath, x: u16, y: usize) -> u16 {
    // TODO        todo!()
    // TODO    }
    // TODO    fn coordinates_by_index(&self, root: &MessageData, path: &FieldPath, x: u16, y: usize) -> u16 {
    // TODO        todo!()
    // TODO    }

    fn get_screen(&self, root: &MessageData, path: &FieldPath, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines {
        let mut lines = ScreenLines::new();
        let mut line = ScreenLine::new(width);
        if let Some(field_def) = root.get_field_definition(path) {
            line.add_field_name(field_def.name().clone(), indent, &cursor);


            let selected_index = cursor.map_or(usize::MAX, |(x, y)| self.data_index_at_cursor(x, y));

            if self.amount == 0 {
                // no data was read, show default value
                if let FieldValue::SCALAR(value) = field_def.default() {
                    Self::add_scalar_value(&mut line, &value, &field_def, config, selected_index == 0);
                }
            } else {
                let mut avail_width = (width - indent - Self::MARGIN) as usize;
                if field_def.repeated() { avail_width -= 1 }
                avail_width -= field_def.typename().len();

                debug_assert!(self.amount > 0);
                let mut cur_len = 0;
                let mut line_count = 1;
                let mut p = path.0.clone();
                for index in 0..self.amount {
                    if let Some(field) = root.get_field(&p) {
                        if let FieldValue::SCALAR(value) = &field.value {
                            let str_value = Self::scalar_to_string(value, &field_def, config);
                            let len = str_value.len();
                            cur_len += len + 1;
                            if cur_len >= avail_width {
                                cur_len = len + 1;
                                line_count += 1;

                                if lines.0.is_empty() {
                                    avail_width = (width - indent - Self::MARGIN) as usize;
                                    line.add_typename(field.def.clone(), width, false);
                                }

                                lines.0.push(line);
                                line = ScreenLine::new(width);
                                line.add_value_address(format!("{}", index), indent, &cursor, lines.0.len());
                            }
                            Self::add_scalar_value(&mut line, value, &field.def, config, selected_index == index);
                        }
                    }
                    p.last_mut().unwrap().index += 1;
                }
            }

            if lines.0.is_empty() {
                line.add_typename(field_def.clone(), width, self.amount == 0);
            }
            line.fix_length(width);
        }

        lines.0.push(line);
        lines
    }
    fn on_command(&mut self, root: &MessageData, path: &FieldPath, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, cursor_x: &mut u16, cursor_y: &mut usize) -> CommandResult
    {
        match command {
            UserCommand::DeleteData => {
                let index = self.data_index_at_cursor(*cursor_x, *cursor_y);
                if self.amount > 0 && index > 0 && index == self.amount - 1 {
                    // correct cursor position after deleting last data item
                    (*cursor_x, *cursor_y) = self.cursor_at_data_index(index - 1);
                }
                let path = path.with_last_index(path.0.last().unwrap().index + index);
                self.line_lens.clear();
                CommandResult::ChangeData(Change { path, action: ChangeType::Delete })
            }
            UserCommand::InsertData => {
                let index = self.data_index_at_cursor(*cursor_x, *cursor_y);
                let path = path.with_last_index(path.0.last().unwrap().index + index + 1);
                (*cursor_x, *cursor_y) = self.cursor_at_data_index(index + 1);
                self.line_lens.clear();
                let def = root.get_field_definition(&path).unwrap();
                CommandResult::ChangeData(Change { path: path.clone(), action: ChangeType::Insert(def.default()) })
            }
            UserCommand::ScrollHorizontally(delta) => {
                if let Some(len) = self.line_lens.get(*cursor_y) {
                    if delta > 0 {
                        *cursor_x = (*cursor_x + delta as u16).min(*len as u16);
                    } else { // delta < 0
                        let delta = (-delta as u16).min(*cursor_x);
                        *cursor_x -= delta;
                    }
                    CommandResult::Redraw
                } else { CommandResult::None }
            }
            UserCommand::Home => {
                *cursor_x = if *cursor_x == 1 { 0 } else { 1 };
                CommandResult::Redraw
            }
            UserCommand::End => {
                if let Some(len) = self.line_lens.get(*cursor_y) {
                    *cursor_x = *len as u16;
                }
                CommandResult::Redraw
            }
            _ => CommandResult::None
        }
    }
}

impl StringLayout {
    const MARGIN: u16 = 8 + MARGIN_LEFT + MARGIN_RIGHT;
    fn get_lines_formated<'t>(&self, full_width: u16, indent: u16, repeated: bool, text: &'t String) -> Vec<(&'t str, bool)> {
        let mut res = vec![];

        let mut avail_width = (full_width - indent - Self::MARGIN) as usize;
        if repeated { avail_width -= 1 }
        if !self.has_value { avail_width -= 1 }

        for line in text.lines() {
            let mut start_pos = 0;
            let mut end_pos = line.len();
            loop {
                if avail_width < end_pos - start_pos {
                    end_pos = start_pos + avail_width;
                }

                // byte index 76 is not a char boundary; it is inside 'а' (bytes 75..77) of `исполняющий обязанности премьер-министра` note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
                res.push((&line[start_pos..end_pos], start_pos == 0));
                avail_width = (full_width - indent - 3) as usize;

                if end_pos >= line.len() { break; }
                start_pos = end_pos;
                end_pos = line.len();
            }
        }
        res
    }
}
impl ViewLayout for StringLayout {
    fn layout_type(&self) -> LayoutType { LayoutType::Str }
    fn amount(&self) -> usize { if self.has_value { 1 } else { 0 } }
    fn calc_sizes(&mut self, root: &MessageData, path: &FieldPath, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) -> usize {

        // calculate width of first column as maximum length of field name and address
        let mut def: Option<FieldProtoPtr> = None;
        let mut value: Option<&String> = None;
        if let Some(field) = root.get_field(&path.0) {
            if let FieldValue::SCALAR(ScalarValue::STR(data)) = &field.value {
                def = Some(field.def.clone());
                value = Some(&data);
            }
        }
        if def.is_none() { // no data was read, get field name from proto file
            if let Some(field_def) = root.get_field_definition(path) {
                def = Some(field_def.clone());
            }
        }

        let mut line_count = 1;
        if let Some(field_def) = def {
            let indent = negotiator.add(field_def.name().len(), path.0.len());

            if let Some(text) = value {
                line_count = self.get_lines_formated(width, indent, field_def.repeated(), text).len();

                let mut address_len = 0;
                address_len = format!("{}", line_count).len() as u16;

                if address_len > indent {
                    negotiator.add(address_len as usize, path.0.len());
                    line_count = self.get_lines_formated(width, indent, field_def.repeated(), text).len();
                    // if line count changed, address length may be increased
                }
            }
        }
        return line_count.max(1);
    }

    fn get_screen(&self, root: &MessageData, path: &FieldPath, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines {
        let mut lines = vec![];
        let mut line = ScreenLine::new(width);

        if let Some(field_def) = root.get_field_definition(path) {
            line.add_field_name(field_def.name().clone(), indent, &cursor);

            if let Some(field) = root.get_field(&path.0) {
                if let FieldValue::SCALAR(ScalarValue::STR(value)) = &field.value {
                    let line_by_line = self.get_lines_formated(width, indent, field_def.repeated(), value);
                    if line_by_line.len() <= 1 {
                        line.0.push((' ', TextStyle::Divider));
                        line.0.push(('\'', TextStyle::Divider));
                        line.add_string(value.to_string(), TextStyle::Value);
                        line.0.push(('\'', TextStyle::Divider));
                        line.fix_length(width);
                    } else { // multiline
                        let mut index = 0;
                        for text in line_by_line {
                            if index > 0 {
                                lines.push(line);
                                line = ScreenLine::new(width);
                                line.add_value_address(
                                    if text.1 {
                                        format!("{}", index + 1) // line after CR/LF
                                    } else {
                                        String::new() // line limited by length
                                    }, indent, &cursor, lines.len());
                            }
                            line.0.push((' ', TextStyle::Divider));
                            line.add_string(text.0.to_string(), TextStyle::Value);
                            line.fix_length(width);
                            if text.1 { index += 1 }
                        }
                    }
                }
            } else {
                line.0.push((' ', TextStyle::Divider));
                line.0.push(('\'', TextStyle::Divider));
                line.0.push(('\'', TextStyle::Divider));
            }
            lines.push(line);
            lines.first_mut().unwrap().add_typename(field_def, width, !self.has_value);
        }
        ScreenLines(lines)
    }
    fn on_command(&mut self, root: &MessageData, path: &FieldPath, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, cursor_x: &mut u16, cursor_pos: &mut usize) -> CommandResult
    {
        //        if let Some(field) = root.get_field(&path.0) {
        //            if let FieldValue::SCALAR(ScalarValue::STR(value)) = &field.value {
        //                self.visible_lines_count = self.get_lines_formated(width, indent, field.def.repeated(), value).len();
        //            }
        //        }
        //        if self.visible_lines_count < 1 { self.visible_lines_count = 1 }

        match command {
            _ => CommandResult::None  // TODO
        }
    }
}

impl BytesLayout {
    fn calc_sizes_internal(&self, mut width: u16, indent: u16, repeated: bool) -> (usize, u16) {
        let mut free_width = width;
        free_width -= indent + 1; // field and ':'
        free_width -= 5; // "bytes".len()
        if !self.has_value { free_width -= 1 } // '-' before type name
        if repeated { free_width -= 1 } // '*' after type name

        let mut blocks_count = free_width / (8 * 3 + 1); // each block 8 bytes wide

        if blocks_count > 0 { // spaces between blocks
            free_width -= (blocks_count - 1);
            blocks_count = free_width / (8 * 3 + 1);
        }

        let bytes_on_line =
            if blocks_count == 0 {
                debug_assert!((free_width - 1) / 3 < 8);
                (free_width - 1) / 3
            } else {
                // if possible, concatenate the last short line with the first line
                if self.data_size as u16 > blocks_count * 8 {
                    let one_line_len = blocks_count * (8 * 3 + 1) + 1 + (self.data_size as u16 - blocks_count * 8) * 3;
                    if one_line_len <= free_width {
                        self.data_size as u16
                    } else { blocks_count * 8 }
                } else { blocks_count * 8 }
            }.max(1);

        // now we can calculate required number of lines
        let mut height = self.data_size / bytes_on_line as usize;
        if self.data_size != height * bytes_on_line as usize {
            height += 1;
        }
        height = height.max(1); // one line always shown, even if there is no data

        (height, bytes_on_line)
    }

    fn data_index_from_cursor(&self, cursor_x: u16, cursor_y: usize) -> Option<usize> {
        if cursor_x == 0 { None } else {
            Some(cursor_x as usize + self.bytes_per_line as usize * cursor_y - 1)
        }
    }

    fn cursor_from_data_index(&self, index: usize) -> (u16, usize) {
        let y = index / self.bytes_per_line as usize;
        let x = index % self.bytes_per_line as usize;
        (x as u16 + 1, y)
    }
}

impl ViewLayout for BytesLayout {
    fn layout_type(&self) -> LayoutType { LayoutType::Bytes }
    fn amount(&self) -> usize { if self.has_value { 1 } else { 0 } }
    fn calc_sizes(&mut self, root: &MessageData, path: &FieldPath, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) -> usize {

        // calculate width of first column as maximum length of field name and address
        let mut name_len = 0;
        let mut address_len = 0;
        self.data_size = 0;
        let mut repeated = false;
        if let Some(field) = root.get_field(&path.0) {
            debug_assert!(self.has_value);
            if let FieldValue::SCALAR(ScalarValue::BYTES(data)) = &field.value {
                self.data_size = data.len();
                address_len = format!("{:x}", self.data_size).len();
                name_len = field.def.name().len();
                repeated = field.def.repeated();
                debug_assert!(name_len > 0);
            }
        }
        if name_len == 0 { // no data was read, get field name from proto file
            if let Some(field_def) = root.get_field_definition(path) {
                name_len = field_def.name().len();
                repeated = field_def.repeated();
            }
        }
        let indent = negotiator.add(address_len.max(name_len), path.0.len());
        let (height, len) = self.calc_sizes_internal(width, indent, repeated);
        self.bytes_per_line = len;
        height
    }

    fn get_screen(&self, root: &MessageData, path: &FieldPath, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines {
        let mut lines = vec![];
        let mut line = ScreenLine::new(width);

        let selected_index = cursor.map_or(usize::MAX, |(x, y)| {
            self.data_index_from_cursor(x, y).unwrap_or(usize::MAX)
        });

        if let Some(field_def) = root.get_field_definition(path) {
            line.add_field_name(field_def.name().clone(), indent, &cursor);

            if let Some(field) = root.get_field(&path.0) {
                if let FieldValue::SCALAR(BYTES(value)) = &field.value {
                    for index in 0..value.len() {
                        if 0 != index {
                            if 0 == index % self.bytes_per_line as usize { // create new line
                                line.fix_length(width);
                                lines.push(line);
                                line = ScreenLine::new(width);
                                line.add_value_address(format!("{:X}", index), indent, &cursor, lines.len());
                            } else { // add space between every 8 bytes
                                if self.bytes_per_line > 8 && 0 == index & 7 { line.add_string(" ".to_string(), TextStyle::Value) }
                            }
                        }
                        let style = if selected_index == index { TextStyle::SelectedValue } else { TextStyle::Value };
                        line.add_string(" ".to_string(), TextStyle::Divider);
                        line.add_string(format!("{:02X}", value[index]), style);
                    }
                }
            }
            line.fix_length(width);
            lines.push(line);
            lines.first_mut().unwrap().add_typename(field_def, width, !self.has_value);
        }
        ScreenLines(lines)
    }

    fn on_command(&mut self, root: &MessageData, path: &FieldPath, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, cursor_x: &mut u16, cursor_y: &mut usize) -> CommandResult {
        match command {
            UserCommand::DeleteData => {
                if let Some(field) = root.get_field(&path.0) {
                    if let FieldValue::SCALAR(BYTES(value)) = &field.value {
                        if let Some(index) = self.data_index_from_cursor(*cursor_x, *cursor_y) {
                            let mut value = value.clone();
                            value.remove(index);
                            self.data_size = value.len();
                            if self.data_size > 0 {
                                (*cursor_x, *cursor_y) = self.cursor_from_data_index(index.min(self.data_size - 1));
                            } else { *cursor_x = 0 }
                            return CommandResult::ChangeData(Change { path: path.clone(), action: ChangeType::Overwrite(FieldValue::SCALAR(BYTES(value))) });
                        }
                    }
                }
                CommandResult::None
            }

            UserCommand::InsertData => {
                if let Some(field) = root.get_field(&path.0) {
                    if let FieldValue::SCALAR(BYTES(value)) = &field.value {
                        if let Some(index) = self.data_index_from_cursor(*cursor_x, *cursor_y) {
                            let mut value = value.clone();
                            value.insert(index + 1, 0);
                            (*cursor_x, *cursor_y) = self.cursor_from_data_index(index + 1);
                            return CommandResult::ChangeData(Change { path: path.clone(), action: ChangeType::Overwrite(FieldValue::SCALAR(BYTES(value))) });
                        }
                    }
                }
                CommandResult::None
            }

            UserCommand::ScrollHorizontally(delta) => {
                if delta > 0 {
                    *cursor_x = (*cursor_x + delta as u16).min(self.bytes_per_line);
                    if *cursor_x as usize + *cursor_y * self.bytes_per_line as usize > self.data_size {
                        *cursor_x = (self.data_size % self.bytes_per_line as usize) as u16;
                    }
                } else { // delta < 0
                    let delta = (-delta as u16).min(*cursor_x);
                    *cursor_x -= delta;
                }
                CommandResult::Redraw
            }

            UserCommand::Home => {
                *cursor_x = if *cursor_x == 1 { 0 } else { 1 };
                CommandResult::Redraw
            }

            UserCommand::End => {
                *cursor_x = self.bytes_per_line;
                let index = self.data_index_from_cursor((*cursor_x).max(1), *cursor_y).unwrap();
                (*cursor_x, *cursor_y) = self.cursor_from_data_index(index.min(self.data_size - 1));
                CommandResult::Redraw
            }

            _ => CommandResult::None
        }
    }

    fn get_consumed_fields(&self, root: &MessageData, path: &FieldPath, config: &LayoutConfig) -> HashSet<i32> {
        todo!()
    }

    fn get_status_string(&self, cursor_x: u16, cursor_y: usize) -> String {
        self.data_index_from_cursor(cursor_x, cursor_y).map_or(String::new(), |index|format!("{}/{}", index, self.data_size))
    }

}

impl MessageLayout {
    fn new(amount: usize) -> Self {
        MessageLayout { amount, scroll: 0 }
    }
}
impl ViewLayout for MessageLayout {
    fn layout_type(&self) -> LayoutType { LayoutType::Message }
    fn amount(&self) -> usize { self.amount }
    fn calc_sizes(&mut self, root: &MessageData, path: &FieldPath, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) -> usize {
        if let Some(field_def) = root.get_field_definition(path) {
            negotiator.add(field_def.name().len(), path.0.len());
        }
        return 1;
    }
    fn get_screen(&self, root: &MessageData, path: &FieldPath, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines {
        debug_assert!(self.amount <= 1);
        let mut line = ScreenLine::new(width);
        if let Some(field_def) = root.get_field_definition(path) {
            line.add_field_name(field_def.name().clone(), indent, &cursor);
            line.add_typename(field_def, width, self.amount == 0);
        }
        ScreenLines(vec![line])
    }
    fn on_command(&mut self, root: &MessageData, path: &FieldPath, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, cursor_x: &mut u16, cursor_pos: &mut usize) -> CommandResult
    {
        match command {
            //UserCommand::TableTreeToggle => { CommandResult::ChangeLayout(LayoutType::Table) }
            _ => CommandResult::None //todo!()
        }
    }
}

impl TableLayout {
    fn new(path: FieldPath, amount: usize) -> Self {
        TableLayout { amount, vertical: false, scroll: (0, 0) }
    }
}
impl ViewLayout for TableLayout {
    fn layout_type(&self) -> LayoutType { LayoutType::Table }
    fn amount(&self) -> usize { self.amount }
    fn calc_sizes(&mut self, root: &MessageData, path: &FieldPath, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) -> usize {
        todo!()
    }
    fn get_screen(&self, root: &MessageData, path: &FieldPath, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines {
        let mut line = ScreenLine::new(width);
        if let Some(field) = root.get_field(&path.0) {
            line.add_field_name(field.def.name().clone(), indent, &cursor);
            line.add_typename(field.def.clone(), width, self.amount == 0);
        }
        ScreenLines(vec![line])
    }
    fn on_command(&mut self, root: &MessageData, path: &FieldPath, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, cursor_x: &mut u16, cursor_pos: &mut usize) -> CommandResult
    {
        match command {
            _ => CommandResult::None //todo!()
        }
    }
}

impl ViewLayout for CollapsedLayout {
    fn layout_type(&self) -> LayoutType { LayoutType::Collapsed }
    fn amount(&self) -> usize { todo!() }
    fn calc_sizes(&mut self, root: &MessageData, path: &FieldPath, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) -> usize {
        let def = root.get_field_definition(path).unwrap();
        negotiator.add(def.name().len(), path.0.len());
        return 1;
    }
    fn get_screen(&self, root: &MessageData, path: &FieldPath, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines {
        let mut line = ScreenLine::new(width);

        if let Some(field_def) = root.get_field_definition(path) {
            line.add_field_name(field_def.name().clone(), indent, &cursor);
            line.add_field_size(self.display_size, width);
            line.add_typename(field_def.clone(), width, self.display_size == 0);
        }


        //        if let Some(field) = root.get_field(&path.0) {
        //            line.add_field_name(field.def.name().clone(), indent, &cursor);
        //            line.add_field_size(self.display_size, width);
        //            line.add_typename(field.def.clone(), width, self.display_size == 0);
        //        }
        ScreenLines(vec![line])
    }
    fn on_command(&mut self, root: &MessageData, path: &FieldPath, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, cursor_x: &mut u16, cursor_pos: &mut usize) -> CommandResult {
        match command {
            _ => CommandResult::None //todo!()
        }
    }

    fn get_consumed_fields(&self, root: &MessageData, path: &FieldPath, config: &LayoutConfig) -> HashSet<i32> {
        if let Some(msg) = root.get_submessage(&path.0) {
            return msg.def.fields.iter().map(|field| field.id()).collect();
        }
        unreachable!()
    }
}

impl TextStyle {
    pub fn first_column(&self) -> bool {
        match self {
            TextStyle::FieldName |
            TextStyle::FieldIndex |
            TextStyle::SelectedFieldIndex |
            TextStyle::SelectedFieldName => true,
            _ => false,
        }
    }

    pub fn activate(&self) -> impl crossterm::Command {

        // color theme may use 16 color, 256 color or true color mode,
        // different modes compatible with different terminals

        let foreground_color = match self {
            TextStyle::TopLine => Color::Black,
            TextStyle::FieldName => Color::Green,
            TextStyle::SelectedValue |
            TextStyle::SelectedFieldIndex |
            TextStyle::SelectedFieldName => Color::Black,
            TextStyle::FieldIndex |
            TextStyle::Divider => Color::DarkGrey,
            TextStyle::Value => Color::White, // Color::AnsiValue(230), // https://www.ditig.com/256-colors-cheat-sheet
            TextStyle::DefaultValue => Color::Grey,
            TextStyle::Typename => Color::DarkCyan,
            TextStyle::Bookmark => Color::Black,
            TextStyle::Unknown => Color::Reset,
            _ => Color::Grey,
        };

        let background_color = match self {
            TextStyle::TopLine => Color::DarkCyan,
            TextStyle::SelectedValue |
            TextStyle::SelectedFieldName |
            TextStyle::SelectedFieldIndex |
            TextStyle::SelectedTypename => Color::DarkCyan,
            TextStyle::Bookmark => Color::Yellow,
            _ => Color::Reset,
        };

        style::SetColors(style::Colors {
            foreground: Some(foreground_color),
            background: Some(background_color),
        })
    }
}

impl LayoutParams {
    pub fn new(path: FieldPath, layout: Box<dyn ViewLayout>) -> LayoutParams {
        LayoutParams { height: 1, path, layout: Some(layout), sibling_count: 0, sibling_index: 0 }
    }
    pub fn new_empty(path: FieldPath) -> LayoutParams {
        LayoutParams { height: 1, path, layout: None, sibling_count: 0, sibling_index: 0 }
    }
    pub fn level(&self) -> usize {
        self.path.0.len()
    }
    pub fn get_status_string(&self, cursor_x: u16, cursor_y: usize) -> String {
        if let Some(layout) = self.layout.as_ref() {
            return layout.get_status_string(cursor_x, cursor_y);
        }
        String::new()
    }
    pub fn calc_sizes(&mut self, root: &MessageData, config: &LayoutConfig, width: u16, negotiator: &mut IndentsCalc) {
        if let Some(layout) = &mut self.layout {
            self.height = layout.as_mut().calc_sizes(root, &self.path, config, width, negotiator);
        }
    }

    pub fn get_screen(&self, root: &MessageData, width: u16, indent: u16, config: &LayoutConfig, cursor: Option<(u16, usize)>) -> ScreenLines
    {
        if let Some(layout) = &self.layout {
            layout.get_screen(root, &self.path, width, indent, config, cursor)
        } else {
            debug_assert!(false);
            ScreenLines::new()
        }
    }

    pub fn on_command(&mut self, root: &MessageData, command: UserCommand, config: &LayoutConfig, width: u16, indent: u16, cursor_x: &mut u16, cursor_pos: &mut usize) -> CommandResult {
        if let Some(layout) = &mut self.layout {
            match command {
                _ => layout.on_command(root, &self.path, command, config, width, indent, cursor_x, cursor_pos),
            }
        } else { CommandResult::None }
    }
}

impl Layouts {
    pub fn new(root: &MessageData, config: &LayoutConfig, opened_file_name: String, width: u16, height: u16) -> Layouts {
        let mut items: Vec<LayoutParams> =
            root.get_sorted_fields(&config.field_order).into_iter().
                map(|pos_ex| Self::create_field_layouts(root, &config, &FieldPath([pos_ex.0].into()), pos_ex.1, false)).
                flatten().collect();

        let mut negotiator = IndentsCalc::new();

        for item in &mut items {
            item.calc_sizes(root, config, width, &mut negotiator); // for scalar field only, messages are empty
        }

        let mut layouts = Layouts { items, file_name: opened_file_name, indents: negotiator.level_indents, scroll: 0, width, height };
        layouts.update_indexes_sibling(0);
        layouts
    }

    fn create_field_layouts(root: &MessageData, config: &LayoutConfig, path: &FieldPath, amount: usize, load_all: bool) -> Vec<LayoutParams> {
        let mut items: Vec<LayoutParams> = vec![];
        let last_pos = path.0.last().unwrap().clone();
        if let Some(field) = root.get_field(&path.0) {
            match &field.value {
                FieldValue::MESSAGE(msg) => {
                    if amount == 0 {
                        items.append(&mut Self::create_message_layouts(root, config, path, amount, load_all));
                    } else {
                        for index in last_pos.index..last_pos.index + amount { // message layout does not support repeated data
                            items.append(&mut Self::create_message_layouts(root, config, &path.with_last_index(index), 1, load_all));
                        }
                    }
                }
                FieldValue::SCALAR(scalar) => {
                    items.append(&mut Self::create_scalar_layouts(field.def.clone(), path.clone(), amount));
                }
            }
        } else { // no data was read, show empty field
            let field_def = root.get_field_definition(&path).unwrap();
            debug_assert!(amount == 0);
            if field_def.is_message() {
                items.append(&mut Self::create_message_layouts(root, config, path, amount, load_all));
            } else {
                items.append(&mut Self::create_scalar_layouts(field_def, path.clone(), amount));
            }
        }
        items
    }

    pub fn create_message_layouts(root: &MessageData, config: &LayoutConfig, path: &FieldPath, amount: usize, load_all: bool) -> Vec<LayoutParams> {
        let mut items: Vec<LayoutParams> = vec![];
        if load_all {
            let msg_layout = MessageLayout::new(amount);
            let consumed_fields = msg_layout.get_consumed_fields(root, path, config);
            items.push(LayoutParams::new(path.clone(), Box::new(msg_layout)));
            if amount > 0 {
                let msg = root.get_submessage(&path.0).unwrap();
                for (pos, amount) in msg.get_sorted_fields(&config.field_order) {
                    if !consumed_fields.contains(&pos.id) {
                        items.append(&mut Self::create_field_layouts(root, config, &path.add(pos), amount, load_all));
                    }
                }
            }
        } else {
            items.push(LayoutParams::new_empty(path.clone()));
        }
        items
    }

    fn create_scalar_layouts(field_def: FieldProtoPtr, path: FieldPath, amount: usize) -> Vec<LayoutParams> {
        let mut items: Vec<LayoutParams> = vec![];
        match field_def.typename().as_str() {
            // repeated strings and bytes always shown separately
            "bytes" => {
                let start = path.0.last().unwrap().index;
                for index in start..start + amount.max(1) {
                    items.push(LayoutParams::new(path.with_last_index(index), Box::new(BytesLayout {
                        has_value: amount != 0,
                        bytes_per_line: 0,
                        data_size: 0,
                    })))
                }
            }
            "string" => {
                let start = path.0.last().unwrap().index;
                for index in start..start + amount.max(1) {
                    items.push(LayoutParams::new(path.with_last_index(index), Box::new(StringLayout {
                        has_value: amount != 0,
                        visible_lines_count: 0,
                    })))
                }
            }
            _ => items.push(LayoutParams::new(path, Box::new(ScalarLayout::new(amount)))),
        }
        items
    }

    pub fn start_indent_update(&mut self) -> IndentsCalc {
        let indents = mem::replace(&mut self.indents, vec![]);
        IndentsCalc::new_for_update(indents)
    }

    pub fn update_layouts(&mut self, root: &MessageData, config: &LayoutConfig) {
        let mut negotiator = self.start_indent_update();
        for item in &mut self.items {
            item.calc_sizes(root, config, self.width, &mut negotiator);
        }
        self.indents = negotiator.into();
    }


    pub fn ensure_loaded(&mut self, root: &MessageData, config: &LayoutConfig, layout_index: usize, lines_before: usize, lines_after: usize, selection: &mut Selection) {
        let mut remain = lines_after as isize;
        let mut i = layout_index;
        while i < self.items.len() {
            //
            if self.items[i].layout.is_some() {
                let mut indent_calc = self.start_indent_update();
                let item = &mut self.items[i];
                item.calc_sizes(root, config, self.width, &mut indent_calc);
                self.indents = indent_calc.into();
                remain -= item.height as isize;
                i += 1;
            } else {
                let (count, lines_count) = self.expand_collapsed(root, config, i);
                remain -= lines_count as isize;
                i += count;
            }
            if remain <= 0 { break; }
        }

        remain = lines_before as isize;
        let mut i = layout_index;
        while i > 0 {
            i -= 1; // [i=0] already processed above
            if self.items[i].layout.is_some() {
                let mut indent_calc = self.start_indent_update();
                let item = &mut self.items[i];
                item.calc_sizes(root, config, self.width, &mut indent_calc);
                self.indents = indent_calc.into();
                remain -= item.height as isize;
            } else {
                let (count, lines_count) = self.expand_collapsed(root, config, i);
                remain -= lines_count as isize;
                if selection.layout > i {
                    selection.layout += count;
                }
            }
            if remain <= 0 { break; }
        }
    }


    fn get_children_count(&self, parent_pos: usize) -> usize {
        if let Some(current) = self.items.get(parent_pos) {
            let path_len = current.path.0.len();
            let mut end_pos = parent_pos + 1;
            while end_pos < self.items.len() {
                let len = self.items[end_pos].path.0.len();
                if len <= path_len { break; }
                end_pos += 1;
            }
            return end_pos - parent_pos;
        }
        0
    }


    // restore message layout with children
    // return a new count of layouts (instead of 1 before) and total lines in them
    fn expand_collapsed(&mut self, root: &MessageData, config: &LayoutConfig, pos: usize) -> (usize, usize) {
        let mut new_layout_count = 0;
        let mut new_lines_count = 0;
        let mut path = None;
        if let Some(current) = self.items.get(pos) {
            path = Some(current.path.clone());
        }
        if let Some(path) = path {
            let mut negotiator = self.start_indent_update();
            let amount = if root.get_field(&path.0).is_some() { 1 } else { 0 };
            let mut layouts = Self::create_message_layouts(root, config, &path, amount, true);
            new_layout_count = layouts.len();
            self.items.remove(pos);
            while !layouts.is_empty() {
                let mut new_item = layouts.pop().unwrap();
                new_item.calc_sizes(root, config, self.width, &mut negotiator);
                new_lines_count += new_item.height;
                self.items.insert(pos, new_item);
            }
            self.indents = negotiator.into();
        }
        self.update_indexes_sibling(pos);
        self.update_indexes_descendant(pos);
        debug_assert!(new_layout_count > 0);
        debug_assert!(new_lines_count > 0);
        (new_layout_count, new_lines_count)
    }


    // update sibling_index and sibling_count of all the descendant of the layout at the pos
    fn update_indexes_descendant(&mut self, pos: usize) {
        if let Some(current) = self.items.get(pos) {
            let min_len = current.path.0.len();

            let mut children_indexes = vec![];
            for i in pos + 1..self.items.len() {
                let len = current.path.0.len();
                if len < min_len { break; } // next sibling or parent
                if len + 1 == min_len { children_indexes.push(i); }
            }

            let sibling_count = children_indexes.len();
            for i in children_indexes {
                self.items[i].sibling_index = i;
                self.items[i].sibling_count = sibling_count;
                self.update_indexes_descendant(i);
            }
        }
    }

    // update sibling_index and sibling_count of the layout at the pos
    fn update_indexes_sibling(&mut self, pos: usize) {
        if let Some(current) = self.items.get(pos) {
            let min_len = current.path.0.len();

            let mut siblings_indexes = vec![];
            for i in pos..0 { // gather siblings upward
                let len = current.path.0.len();
                if len < min_len { break; }
                if len == min_len { siblings_indexes.push(i); }
            }

            for i in pos + 1..self.items.len() { // gather siblings downward
                let len = current.path.0.len();
                if len < min_len { break; }
                if len == min_len { siblings_indexes.push(i); }
            }

            let sibling_count = siblings_indexes.len();
            for i in siblings_indexes {
                self.items[i].sibling_index = i;
                self.items[i].sibling_count = sibling_count;
            }
        }
    }

    pub fn get_parent_pos(&self, mut pos: usize) -> Option<usize> {
        if let Some(current) = self.items.get(pos) {
            let parent_len = current.path.0.len() - 1;
            while pos > 0 {
                pos -= 1;
                let len = self.items[pos].path.0.len();
                if len == parent_len { return Some(pos); }
            }
        }
        None
    }

    pub fn update_after_data_changed(&mut self, root: &MessageData, config: &LayoutConfig, changed_layout: usize) {
        let mut negotiator = self.start_indent_update();

        // when a field changed, recreate layout of the parent message.
        // the field may be repeated, so delete/create it may influence siblings
        if let Some(parent_pos) = self.get_parent_pos(changed_layout) {
            let children_count = self.get_children_count(parent_pos);

            if let Some(parent) = self.items.get(parent_pos) {
                if let Some(parent_msg) = root.get_submessage(&parent.path.0) {
                    let mut layouts = Self::create_message_layouts(root, config, &parent.path, 1, true);
                    self.items.drain(parent_pos..parent_pos + children_count);
                    while !layouts.is_empty() {
                        let mut new_item = layouts.pop().unwrap();
                        new_item.calc_sizes(root, config, self.width, &mut negotiator);
                        self.items.insert(parent_pos, new_item);
                    }
                }
            }
        } else { // if changed a field of the root message, rebuild all layouts
            let mut items: Vec<LayoutParams> =
                root.get_sorted_fields(&config.field_order).into_iter().
                    map(|pos_ex| Self::create_field_layouts(root, &config, &FieldPath([pos_ex.0].into()), pos_ex.1, true)).
                    flatten().collect();

            for item in &mut items {
                item.calc_sizes(root, config, self.width, &mut negotiator);
            }
            self.items = items;
        }
        self.indents = negotiator.into();
    }
    fn run_command_current_layout(&mut self, command: UserCommand, root: &MessageData, config: &LayoutConfig, selection: &mut Selection) -> CommandResult {
        if let Some(current) = self.items.get_mut(selection.layout) {
            let indent = self.indents[current.level() - 1 as usize];
            current.on_command(root, command, config, self.width, indent, &mut selection.x, &mut selection.y)
        } else {
            CommandResult::None
        }
    }
    pub fn run_command(&mut self, command: UserCommand, root: &MessageData, config: &LayoutConfig, selection: &mut Selection) -> CommandResult {
        match &command {
            UserCommand::ScrollVertically(mut delta, move_up) => {
                if delta == 0 { return CommandResult::None; }
                let mut from_beneath = false;

                loop {
                    if let Some(current) = self.items.get(selection.layout) {
                        debug_assert!(current.layout.is_some());
                        if !*move_up { // cursor moving down
                            if selection.y + delta < current.height {
                                selection.y += delta as usize;
                                debug_assert!(selection.y < current.height as usize);
                                break;
                            }
                            delta -= current.height as usize - selection.y;

                            if selection.layout + 1 >= self.items.len() {
                                selection.y = current.height - 1;
                                break;
                            }
                            selection.layout += 1;
                            selection.y = 0;
                        } else { // cursor moving up

                            if from_beneath { selection.y = current.height - 1; }

                            if selection.y >= delta {
                                selection.y -= delta;
                                break;
                            }
                            delta -= (selection.y + 1);

                            if selection.layout == 0 {
                                selection.y = 0;
                                break;
                            }
                            selection.layout -= 1;
                            from_beneath = true;
                        }
                    } else { // if no layouts exists
                        *selection = Selection::default();
                        break;
                    }
                }
                CommandResult::Redraw
            }

            UserCommand::ScrollSibling(delta) => {
                self.scroll_sibling(*delta, selection);
                CommandResult::Redraw
            }

            UserCommand::ScrollToBottom => {
                self.ensure_loaded(root, config, self.items.len() - 1, (2 * self.height + 1) as usize, 0, selection);
                selection.layout = self.items.len() - 1;
                selection.y = self.items[selection.layout].height - 1;
                selection.x = 0;
                CommandResult::Redraw
            }

            UserCommand::DeleteData => {
                if selection.x == 0 && selection.y == 0 {
                    if let Some(current) = self.items.get(selection.layout) {
                        CommandResult::ChangeData(Change { path: current.path.clone(), action: ChangeType::Delete })
                    } else { CommandResult::None }
                } else {
                    self.run_command_current_layout(command, root, config, selection)
                }
            }

            UserCommand::InsertData => {
                if selection.x == 0 && selection.y == 0 {
                    if let Some(current) = self.items.get(selection.layout) {
                        let def = root.get_field_definition(&current.path).unwrap();
                        CommandResult::ChangeData(Change { path: current.path.clone(), action: ChangeType::Insert(def.default()) })
                    } else { CommandResult::None }
                } else {
                    self.run_command_current_layout(command, root, config, selection)
                }
            }

            UserCommand::CollapsedToggle => {
                if let Some(current) = self.items.get(selection.layout) {
                    if let Some(layout) = &current.layout {
                        match layout.layout_type() {
                            LayoutType::Message => {
                                let current_path = current.path.clone();
                                // there is no reason to collapse a message that does not exist, it's already displayed in one line
                                if let Some(msg) = root.get_submessage(&current_path.0) {
                                    // remove selected layout and all nested layouts
                                    let path_len = current.path.0.len();
                                    let mut end_pos = selection.layout + 1;
                                    while end_pos < self.items.len() {
                                        let len = self.items[end_pos].path.0.len();
                                        if len <= path_len { break; }
                                        end_pos += 1;
                                    }
                                    self.items.drain(selection.layout + 1..end_pos);
                                    // create a collapsed layout in place of the deleted
                                    self.items[selection.layout] = LayoutParams::new(current_path, Box::new(CollapsedLayout { display_size: msg.len() }));
                                }
                            }
                            LayoutType::Collapsed => {
                                self.expand_collapsed(root, config, selection.layout);
                            }
                            _ => {}
                        }
                    }
                }
                CommandResult::Redraw
            }
            _ => self.run_command_current_layout(command, root, config, selection)
        }


        //        if let Some(current) = self.items.get_mut(selection.layout) {
        //            let indent = self.indents[current.level() - 1];
        //            current.on_command(root, command, config, self.width, indent, &mut selection.x, &mut selection.y)
        //        } else { CommandResult::None }
    }

    pub fn scroll_sibling(&self, delta: i8, selection: &mut Selection) -> bool {
        assert!(delta == -1 || delta == 1);
        let mut it = self.items.iter();
        let mut pos = 0;
        loop {
            if let Some(item) = it.next() {
                if pos == selection.layout {
                    let level = item.level();
                    loop {
                        let new_pos = pos as isize + delta as isize;
                        if new_pos < 0 || new_pos >= self.items.len() as isize { break; }
                        pos = new_pos as usize;


                        let item_level = self.items[pos].path.0.len();
                        if item_level < level {
                            return false;
                        }
                        if item_level == level {
                            selection.layout = pos;
                            selection.y = 0;
                            return true;
                        }
                    }
                    break;
                }
                pos += 1;
            } else { break; }
        }
        false // sibling field not found
    }
}

impl Debug for ScreenLine {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut first = true;
        for item in &self.0 {
            write!(f, "{}", item.0)?;
        }
        Ok(())
    }
}

impl Debug for ScreenLines {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for line in &self.0 {
            writeln!(f, "{:?}", line)?;
        }
        Ok(())
    }
}
