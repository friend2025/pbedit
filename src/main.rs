#![allow(warnings)]

mod proto;
mod wire;
mod typedefs;
mod editor;
mod view;
mod trz;

use std::string::String;
use crate::ScalarValue::STR;
use std::collections::HashMap;
use crate::ScalarValue::I32;
use std::fmt::{Debug, Formatter};
use wire::*;
use std::io::{self, Read, Stdout, Write};
use std::path::PathBuf;
use std::process::exit;
use crossterm::*;
use crossterm::style::{Color, Colored, Colors, ContentStyle, Stylize};
use crate::view::{CommandResult, CommentVisibility, FieldOrder, LayoutConfig, LayoutType, Layouts, ScreenLine, ScreenLines, IndentsCalc, TextStyle, UserCommand, MARGIN_LEFT, MARGIN_RIGHT};

use clap::Parser;

//#![cfg(feature = "bracketed-paste")]
use crossterm::{
    event::{
        read, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, Event,
    },
    execute,
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use pest::Lines;
use crate::proto::{FieldProtoPtr, MessageProto, ProtoData, ProtoFile};
use crate::typedefs::{PbReader};
use crate::view::UserCommand::{ChangeFieldOrder, CollapsedToggle, DeleteData, End, Home, InsertData, ScrollHorizontally, ScrollSibling, ScrollToBottom, ScrollVertically};
use crate::wire::FieldValue::SCALAR;

const USE_ALTERNATIVE_SCREEN: bool = false;

// 0-hide top line, 1-show
const TOP_LINE: u16 = 1;


struct RepeatedEditorConfig {
    sort_by: Option<i32>, // field index for sort data
    limit: Option<usize>, // lines count available for the editor
    vertical: bool, // field names in the left column
    columns: u16, // 0 to autofill all available space
}

// UpperUilayer: confirmations (CtrlC exit,etc.), enum/oneof lists


#[derive(Default)]
struct Selection {
    // current active layout index
    layout: usize,
    // y position in the layout
    y: usize,
    // x coordinate in the layout
    // 0 if selected the first column with field names
    x: u16,
}

struct App {
    pub stdout: Stdout,
    pub width: u16,
    pub height: u16,
    test_mode: bool,

    //- field below for each opened document

    pub data: MessageData,
    pub layouts: Layouts,
    pub layout_config: LayoutConfig,
    pub selected: Selection,
    pub need_update: bool,
    pub need_update_layout_height: bool,
}

impl App {
    pub fn new(data: MessageData, file_name: PathBuf) -> io::Result<App> {
        let mut stdout = io::stdout();
        crossterm::terminal::enable_raw_mode()?;
        if (USE_ALTERNATIVE_SCREEN) { stdout.execute(EnterAlternateScreen)?; }
        stdout.execute(terminal::Clear(terminal::ClearType::All))?;
        stdout.execute(EnableBracketedPaste)?;
        stdout.execute(EnableFocusChange)?;
        stdout.execute(cursor::Hide)?;
        let layout_config = LayoutConfig::default();

        let mut width = 0;
        let mut height = 0;
        if let Ok(sizes) = terminal::size() {
            width = sizes.0;
            height = sizes.1;
        }

        let mut layouts = Layouts::new(&data, &layout_config, file_name.file_name().unwrap().to_string_lossy().into_owned(), width, height - TOP_LINE);
        layouts.ensure_loaded(&data, &layout_config, 0, 0, height as usize, &mut Selection::default());
        let mut app = App {
            stdout,
            width,
            height,
            data,
            layouts,
            layout_config,
            selected: Selection::default(),
            need_update: true,
            need_update_layout_height: true,
            test_mode: false,
        };
        app.update()?;
        Ok(app)
    }

    #[cfg(test)]
    pub fn for_tests(data: MessageData, field_order: FieldOrder, width: u16, height: u16) -> io::Result<App> {
        let layout_config = LayoutConfig {
            field_order,
            ..LayoutConfig::default()
        };
        let mut layouts = Layouts::new(&data, &layout_config, "test_data.pb".into(), width, height - TOP_LINE);
        layouts.ensure_loaded(&data, &layout_config, 0, 0, height as usize, &mut Selection::default());
        let mut app = App {
            stdout: io::stdout(),
            width,
            height,
            data,
            layouts,
            layout_config,
            selected: Selection::default(),
            need_update: true,
            need_update_layout_height: true,
            test_mode: true,
        };
        app.to_strings();
        Ok(app)
    }
    pub fn run(&mut self) -> io::Result<()> {
        while
        match read()? {
            Event::FocusGained => self.on_focus(true)?,
            Event::FocusLost => self.on_focus(false)?,
            Event::Key(event) => self.on_key(event)?,
            Event::Mouse(event) => self.on_mouse(event)?,
            Event::Resize(width, height) => self.on_resize(width, height)?,
            _ => false,
        } { self.after_event()?; }
        Ok(())
    }
    fn set_sizes(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        self.layouts.height = height - TOP_LINE;
        self.layouts.width = width;
        self.need_update = true;
    }
    fn after_event(&mut self) -> io::Result<()> {
        if self.need_update_layout_height { // after show/hidde comment for example
            self.layouts.update_layouts(&self.data, &self.layout_config);
            self.need_update_layout_height = false;
            self.need_update = true;
        }

        if self.need_update {
            if self.width == 0 || self.height == 0 {
                if let Ok(sizes) = terminal::size() {
                    self.set_sizes(sizes.0, sizes.1);
                }
            }
            if self.width > 0 && self.height > 0 {
                self.layouts.scroll = self.calc_scroll_pos();
                if self.selected.layout >= self.layouts.items.len() {
                    self.selected.layout = self.layouts.items.len().max(1) - 1;
                }
                if !self.test_mode { self.update()?; }
                self.need_update = false;
            }
        }
        Ok(())
    }
    pub fn on_resize(&mut self, width: u16, height: u16) -> io::Result<bool> {
        self.set_sizes(width, height);
        self.stdout.execute(terminal::Clear(terminal::ClearType::All))?;
        Ok(true)
    }
    pub fn on_focus(&mut self, focus: bool) -> io::Result<bool> {
        self.need_update = true;
        Ok(true)
    }
    pub fn on_mouse(&mut self, event: MouseEvent) -> io::Result<bool> {
        match event.kind {
            MouseEventKind::ScrollUp => { self.run_command(ScrollVertically(3, true))?; }
            MouseEventKind::ScrollDown => { self.run_command(ScrollVertically(3, false))?; }
            _ => {}
        }
        Ok(true)
    }
    pub fn on_key(&mut self, event: KeyEvent) -> io::Result<bool> {
        if event.kind != KeyEventKind::Press { return Ok(true); }
        match event.code {
            KeyCode::F(n) => match n {
                4 => {
                    let new_order =
                        if event.modifiers.contains(KeyModifiers::SHIFT) { self.layout_config.field_order.prev() } else { self.layout_config.field_order.next() };
                    self.run_command(ChangeFieldOrder(new_order))?;
                }
                5 => {
                    self.run_command(CollapsedToggle)?;
                }
                6 => {
                    self.layout_config.show_comments = self.layout_config.show_comments.next();
                    self.need_update_layout_height = true;
                }
                10 => return Ok(false),
                _ => {}
            },
            KeyCode::Esc => return Ok(false),
            KeyCode::Enter => self.run_command(CollapsedToggle)?,
            KeyCode::Up => {
                self.run_command(if event.modifiers.contains(KeyModifiers::CONTROL) { ScrollSibling(-1) } else { ScrollVertically(1, true) })?;
            }
            KeyCode::Down => {
                self.run_command(if event.modifiers.contains(KeyModifiers::CONTROL) { ScrollSibling(1) } else { ScrollVertically(1, false) })?;
            }
            KeyCode::PageUp => { self.run_command(ScrollVertically((self.height - TOP_LINE - 1) as usize, true))?; }
            KeyCode::PageDown => { self.run_command(ScrollVertically((self.height - TOP_LINE - 1) as usize, false))?; }
            KeyCode::Home => if event.modifiers.contains(KeyModifiers::CONTROL) {
                self.selected = Selection::default();
                self.need_update = true;
            } else { self.run_command(crate::UserCommand::Home)?; }
            KeyCode::End => self.run_command(if event.modifiers.contains(KeyModifiers::CONTROL) { ScrollToBottom } else { End })?,
            KeyCode::Left => { self.run_command(ScrollHorizontally(-1))?; }
            KeyCode::Right => { self.run_command(ScrollHorizontally(1))?; }

            KeyCode::Delete => self.run_command(DeleteData)?,
            KeyCode::Insert => self.run_command(InsertData)?,
            _ => {}
        }
        Ok(true)
    }

    fn run_command(&mut self, command: UserCommand) -> io::Result<()> {
        let result =
            match command {
                UserCommand::ChangeFieldOrder(order) => {
                    self.layout_config.field_order = order;
                    self.selected = Selection::default();
                    self.need_update_layout_height = true;
                    self.layouts = Layouts::new(&self.data, &self.layout_config, self.layouts.file_name.clone(), self.layouts.width, self.layouts.height);
                    CommandResult::Redraw
                }
                UserCommand::ScrollVertically(delta, move_up) => {
                    if move_up {
                        self.layouts.ensure_loaded(&self.data, &self.layout_config, self.selected.layout, delta + 1 + self.height as usize, 0, &mut self.selected);
                    } else {
                        self.layouts.ensure_loaded(&self.data, &self.layout_config, self.selected.layout, 0, delta + 1, &mut self.selected);
                    }
                    self.layouts.run_command(command, &self.data, &self.layout_config, &mut self.selected)
                }
                _ => self.layouts.run_command(command, &self.data, &self.layout_config, &mut self.selected)
            };

        self.after_command(result)
    }

    fn after_command(&mut self, result: CommandResult) -> io::Result<()> {
        match result {
            CommandResult::Redraw => {
                self.need_update = true;
            }
            CommandResult::ChangeData(mut change) => {
                self.data.apply(&mut change);
                self.layouts.update_after_data_changed(&self.data, &self.layout_config, self.selected.layout);
                self.need_update_layout_height = true;
            }

            _ => {}
        }
        Ok(())
    }

    fn get_position_percent(&self, mut layout_index: usize) -> f32 {
        let mut res = 0.0;
        loop {
            if let Some(layout) = self.layouts.items.get(layout_index) {
                res = res / layout.sibling_count as f32;
                let share = layout.sibling_index as f32 / layout.sibling_count as f32;
                res = res + share;
            }

            if let Some(index) = self.layouts.get_parent_pos(layout_index) {
                layout_index = index;
            } else {
                break;
            }
        }
        res
    }
    fn get_top_line(&self, width: u16, config: &LayoutConfig) -> String {
        let mut parts = Vec::with_capacity(3);

        parts.push(self.layouts.file_name.clone());
        if let Some(current) = self.layouts.items.get(self.selected.layout) {
            debug_assert!(current.layout.is_some());
            parts.push(current.get_status_string(self.selected.x, self.selected.y));
            parts.push(format!("{}/{} |{}", current.sibling_index, current.sibling_count, config.field_order.first_letter()));
        }

        loop {
            let total_len: u16 = parts.iter().map(|s| s.len() as u16).sum();
            if total_len < width - MARGIN_LEFT - MARGIN_RIGHT {
                let avail_len = width - total_len - MARGIN_LEFT - MARGIN_RIGHT;
                let span = avail_len / (parts.len() as u16 - 1);
                let last_span = avail_len - span * (parts.len() as u16 - 2);

                let mut res = " ".repeat(MARGIN_LEFT as usize);
                for i in 0..parts.len() {
                    res += &parts[i];

                    if i < parts.len() - 1 {
                        let span = if i == parts.len() - 2 { last_span } else { span };
                        res += &" ".repeat(span as usize);
                    }
                }

                res += &" ".repeat(MARGIN_RIGHT as usize);
                return res;
            } else {
                match parts.len() { // remove parts of the line if no room
                    3 => { parts.remove(0); }
                    2 => { parts.remove(1); }
                    _ => return String::new(),
                }
            }
        }
    }

    // find out the line number with active cursor
    fn calc_scroll_pos(&self) -> usize { // move to layouts
        let mut selected_line = 0;
        let mut y = 0;
        for index in 0..self.layouts.items.len() {
            let item = &self.layouts.items[index];
            if self.selected.layout == index {
                //-                debug_assert!(self.selected.x == 0); // for other columns algorithm more complex
                selected_line = y + self.selected.y;
                break;
            }
            y += item.height;
        }
        // correct scroll position if active cursor is above/below visible window
        if selected_line + 1 >= self.layouts.scroll + (self.height - TOP_LINE) as usize {
            return selected_line + 1 - (self.height - TOP_LINE) as usize;
        }
        if selected_line < self.layouts.scroll {
            return selected_line;
        }
        self.layouts.scroll
    }

    fn print_top_line(&mut self) -> io::Result<()> {
        if TOP_LINE > 0 {
            let mut last_pos = 0;
            let mut current_pos = 0;
            for index in 0..self.layouts.items.len() {
                let item = &self.layouts.items[index];
                if self.selected.layout == index {
                    current_pos = last_pos + self.selected.y;
                }
                last_pos += item.height;
            }
            self.stdout.queue(TextStyle::TopLine.activate())?;
            self.stdout.queue(style::Print(self.get_top_line(self.width, &self.layout_config)))?;
        }
        Ok(())
    }


    fn first_visible_line(&self) -> (usize, usize) {
        let mut skip_lines = self.layouts.scroll;
        let mut lines_len = 0;
        for layout_index in 0..self.layouts.items.len() {
            let item = &self.layouts.items[layout_index];

            //if item.layout.is_none() { return (0, 0); }
            //            if item.layout.is_none() { panic!("layout is not loaded: {}", layout_index); }
            //            debug_assert!(item.layout.is_some());   // TODO

            debug_assert!(item.height > 0);
            lines_len = item.height;
            if lines_len > skip_lines {
                return (layout_index, skip_lines);
            }
            skip_lines -= lines_len;
        }

        if self.layouts.items.is_empty() {
            (0, 0)
        } else {
            (self.layouts.items.len() - 1, lines_len - 1)
        }
    }

    // output data to the screen
    fn update(&mut self) -> io::Result<()> {
        self.stdout.queue(cursor::MoveTo(0, 0))?;

        let (layout_index, mut skip_lines) = self.first_visible_line();
        self.layouts.ensure_loaded(&self.data, &self.layout_config, layout_index, 0, self.height as usize + skip_lines, &mut self.selected);

        self.print_top_line()?;
        let mut y = TOP_LINE;

        let mut current_style = TextStyle::Unknown;
        for index in layout_index..self.layouts.items.len() {
            let item = &mut self.layouts.items[index];
            let cursor = if index == self.selected.layout { Some((self.selected.x, self.selected.y)) } else { None };
            let indent = self.layouts.indents[item.level() - 1];

            let mut lines = item.get_screen(&self.data, self.layouts.width, indent, &self.layout_config, cursor);

            if skip_lines > 0 {
                lines.0.drain(..skip_lines);
                skip_lines = 0;
            }

            for line in lines.0 {
                let mut text = String::new();
                for (c, s) in line.0 {
                    if s != current_style {
                        if !text.is_empty() {
                            self.stdout.queue(current_style.activate())?;
                            self.stdout.queue(style::Print(text))?;
                            text = String::new();
                        }
                        current_style = s;
                    }
                    text.push(c);
                }
                if !text.is_empty() {
                    self.stdout.queue(current_style.activate())?;
                    self.stdout.queue(style::Print(text))?;
                }
                self.stdout.queue(cursor::MoveToNextLine(1))?;
                y += 1;
                if y >= self.height { break; }
            }
            if y >= self.height { break; }
        }
        if y < self.height { // fill the free space below if any
            self.stdout.queue(style::ResetColor)?;
            self.stdout.execute(terminal::Clear(terminal::ClearType::FromCursorDown))?;
        }
        self.stdout.flush()
    }

    #[cfg(test)]
    fn to_strings(&mut self) -> Vec<String> {
        let mut y = TOP_LINE;
        let mut res = vec![];

        let (layout_index, mut skip_lines) = self.first_visible_line();
        self.layouts.ensure_loaded(&self.data, &self.layout_config, layout_index, 0, self.height as usize + skip_lines, &mut self.selected);

        for index in layout_index..self.layouts.items.len() {
            let item = &self.layouts.items[index];
            let cursor = if index == self.selected.layout { Some((self.selected.x, self.selected.y)) } else { None };
            let indent = self.layouts.indents[item.level() - 1];

            let mut lines = item.get_screen(&self.data, self.layouts.width, indent, &self.layout_config, cursor);

            if skip_lines > 0 {
                lines.0.drain(..skip_lines);
                skip_lines = 0;
            }

            for line in lines.0 {
                res.push(line.0.into_iter().map(|v| v.0).collect());
                y += 1;
                if y >= self.height { break; }
            }
            if y >= self.height { break; }
        }
        res
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if !self.test_mode {
            let _ = self.stdout.execute(DisableBracketedPaste);
            let _ = self.stdout.execute(DisableFocusChange);
            if USE_ALTERNATIVE_SCREEN { let _ = self.stdout.execute(LeaveAlternateScreen); }
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = self.stdout.execute(cursor::Show);
        }
    }
}


/// Protobuf editor
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Input file: data.pb{;format.proto{;message_name}}
    file: String,

    /// Set of directories for proto files search
    #[arg(short='I', long="proto_path")]
    proto_path: Vec<PathBuf>,
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    let mut it = args.file.split(";");
    let binary_file = it.next().unwrap();
    let mut proto_file = String::new();
    let mut root_message_name = String::new();
    if let Some(path) = it.next() {
        proto_file = path.to_string();
        if let Some(path) = it.next() {
            root_message_name = path.to_string();
        }
        assert!(it.next().is_none());
    }

    // if no proto file provided, use the file with the same name as data file but with proto extension
    if proto_file.is_empty() {
        proto_file = binary_file.trim_end_matches(".pb").to_string() + ".proto";
    }

    if !std::fs::exists(&binary_file)? {
        eprintln!("Data file \"{}\" is not available", binary_file);
        exit(101);
    }
    if !std::fs::exists(&proto_file)? {
        eprintln!("Proto definitions file \"{}\" is not available.", proto_file);
        exit(102);
    }

    for dir in &args.proto_path {
        if !dir.is_absolute() {
            eprintln!("The proto_path argument should contain an absolute path.");
            break;
        }
        if !dir.is_dir() {
            eprintln!("The proto_path is not a directory: {}", dir.display());
        }
    }

    let mut proto_files = ProtoFile::new_with_imports(proto_file.into(), args.proto_path);

    let mut proto = ProtoData::new(&proto_files.remove(0).content)?;

    let mut root_msg = None;
    if root_message_name.is_empty() {
        root_msg = proto.auto_detect_root_message(); // search only in the main proto file
        if root_msg.is_none() {
            eprintln!("Cannot choose the root message in the proto definition file; please provide it manually.");
            exit(103);
        }
    }

    // merge imported proto files
    for file in proto_files.into_iter() {
        proto.append(ProtoData::new(&file.content)?);
    }
    proto = proto.finalize()?;

    if root_msg.is_none() {
        root_msg = proto.get_message_definition(&root_message_name);
        if root_msg.is_none() {
            eprintln!("Root message \"{}\" not found.", root_message_name);
            exit(104);
        }
    }

    println!("loading...");
    let file = std::fs::File::open(binary_file)?;
    let mut limit = file.metadata()?.len() as u32;
    let mut reader = PbReader::new(file);
    let data = MessageData::new(&mut reader, &proto, root_msg.unwrap(), &mut limit)?;

    App::new(data, binary_file.into())?.run()
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
mod app_tests {
    use std::path::Iter;
    use super::*;
    use crate::App;
    use crate::proto::ProtoData;
    use crate::wire::FieldValue::MESSAGE;
    use crate::wire::ScalarValue::{BYTES, ENUM, F64, STR};

    fn make_minimal_test_data() -> MessageData {
        let binary_input = [];
        let proto = ProtoData::new("message M { int32 f1 = 1; }").unwrap().finalize().unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.auto_detect_root_message().unwrap();
        let mut read = PbReader::new(binary_input.as_slice());
        MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap()
    }

    fn make_one_field_data(proto: &str, value: ScalarValue) -> MessageData {
        //let binary_input = [];
        //let proto = ProtoData::new(proto).unwrap();
        //let mut limit = binary_input.len() as u32;
        //let root_msg = proto.auto_detect_root_message().unwrap();
        //let mut read = PbReader::new(binary_input.as_slice());
        //let mut data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        let mut data = make_no_field_data(proto);

        let mut field = data.add_field(&[(1, 0).into()]).unwrap();
        field.value = FieldValue::SCALAR(value);

        data
    }

    fn make_no_field_data(proto: &str) -> MessageData {
        let binary_input = [];
        let proto = ProtoData::new(proto).unwrap().finalize().unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.auto_detect_root_message().unwrap();
        let mut read = PbReader::new(binary_input.as_slice());
        let mut data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        data
    }


    fn make_test_data_1() -> MessageData {
        let proto_str = r#"
message M { int32 f1 = 1; repeated int32 f2 = 2; M3 m3 = 3; int32 f4 = 4; }
message M3 { int32 f5 = 5; repeated M6 m6 = 6; int32 f7 = 7; }
message M6 { int32 f8 = 8; int32 f9 = 9; }
"#;

        let binary_input = [
            0x08, 1,  // f1: 1 int32
            0x10, 20,  // f2: 20 int32     repeated scalar
            0x10, 21,  // f2: 21 int32
            //0x20,  4,  // f4: 4 int32    optional scalar
            0x1A, 16,  // m3: M3           nested message
            0x28, 5,  //   f5: 5 int32

            0x32, 4,  //   m6: M6          repeated message
            0x40, 8,  //     f8: 8 int32
            0x48, 9,  //     f9: 9 int32

            0x32, 4,  //   m6: M6
            0x40, 10,  //     f8: 10 int32
            0x48, 11,  //     f9: 11 int32

            0x38, 7,  //   f7: 7 int32
        ];

        let proto = ProtoData::new(proto_str).unwrap().finalize().unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.auto_detect_root_message().unwrap();
        let mut read = PbReader::new(binary_input.as_slice());

        //        let mut f = std::fs::File::create("test_data_1.pb").unwrap();
        //        f.write_all(binary_input.as_slice()).unwrap();

        MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap()
        //assert_eq!(crate::view::MARGIN_LEFT, 1);
        //assert_eq!(crate::view::MARGIN_RIGHT, 1);
    }


    fn make_repeated_message_data(messages_count: usize) -> MessageData {
        let proto_str = r#"
message M { repeated M2 m1 = 1; }
message M2 { int32 i2 = 2; int32 i3 = 3; }
"#;

        let binary_input = [
            //0x0A, 12, // m1: M2           nested message

            //0x0A, 4,  //   m2: M2          repeated message
            //0x10, 2,  //     i2: 2 int32
            //0x18, 3,  //     i3: 3 int32
            ////
            //0x0A, 4,  //   m2: M2          repeated message
            //0x10, 4,  //     i2: 4 int32
            //0x18, 5,  //     i3: 5 int32
            //
            //0x0A, 4,  //   m2: M2          repeated message
            //0x10, 6,  //     i2: 6 int32
            //0x18, 7,  //     i3: 7 int32
        ];

        let proto = ProtoData::new(proto_str).unwrap().finalize().unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.auto_detect_root_message().unwrap();
        let mut read = PbReader::new(binary_input.as_slice());
        let mut data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        let mut int_value = 2;
        for index in 0..messages_count {
            let mut field = data.add_field(&[(1, index).into()]).unwrap();
            if let MESSAGE(msg) = &mut field.value {
                let mut i2 = msg.add_field(&[(2, 0).into()]).unwrap();
                i2.value = SCALAR(I32(int_value));
                int_value += 1;
                let mut i3 = msg.add_field(&[(3, 0).into()]).unwrap();
                i3.value = SCALAR(I32(int_value));
                int_value += 1;
            }
        }
        data
    }


    #[test]
    fn simple() {
        let mut data = make_one_field_data("message M { int32 i1=1; }", I32(1));
        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        assert_eq!(app.to_strings(), [" i1: 1                  int32 "]);
    }

    #[test]
    fn app_test_1() {
        let data = make_test_data_1();
        let mut app = App::for_tests(data, FieldOrder::Proto, 50, 25).unwrap();
        let expected = [
            " f1: 1                                      int32 ",
            " f2: 20 21                                 int32* ",
            " m3:                                           M3 ",
            "   f5: 5                                    int32 ",
            "   m6:                                        M6* ",
            "     f8: 8                                  int32 ",
            "     f9: 9                                  int32 ",
            "   m6:                                        M6* ",
            "     f8: 10                                 int32 ",
            "     f9: 11                                 int32 ",
            "   f7: 7                                    int32 ",
            " f4: 0                                     -int32 "];
        assert_eq!(app.to_strings(), expected);
    }


    #[test]
    fn scroll_limits() {
        let expected_start = [
            " f1: 1                                      int32 ",
            " f2: 20 21                                 int32* "];
        let expected_end = [
            "   f7: 7                                    int32 ",
            " f4: 0                                     -int32 "];

        let data = make_test_data_1();
        let mut app = App::for_tests(data, FieldOrder::Proto, 50, 2 + TOP_LINE).unwrap();
        assert_eq!(app.to_strings(), expected_start);

        for _ in 0..100 {
            app.run_command(UserCommand::ScrollVertically(1, false)).unwrap();
            app.after_event().unwrap();
        }
        assert_eq!(app.to_strings(), expected_end);

        for _ in 0..100 {
            app.run_command(UserCommand::ScrollVertically(1, true)).unwrap();
            app.after_event().unwrap();
        }
        assert_eq!(app.to_strings(), expected_start);

        for _ in 0..100 {
            app.run_command(UserCommand::ScrollSibling(1)).unwrap();
            app.after_event().unwrap();
        }
        assert_eq!(app.to_strings(), expected_end);

        for _ in 0..100 {
            app.run_command(UserCommand::ScrollSibling(-1)).unwrap();
            app.after_event().unwrap();
        }
        assert_eq!(app.to_strings(), expected_start);

        app.run_command(UserCommand::ScrollVertically(100, false)).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), expected_end);

        app.run_command(UserCommand::ScrollVertically(100, true)).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), expected_start);
    }

    #[test]
    fn empty_repeated_message() {
        let mut data = make_repeated_message_data(0);
        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        assert_eq!(app.to_strings(), [" m1:                     -M2* "]);
    }

    #[test]
    fn insert_repeated_message() {
        let mut data = make_repeated_message_data(0);
        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        assert_eq!(app.to_strings(), [" m1:                     -M2* "]);

        app.run_command(UserCommand::InsertData).unwrap();
        app.after_event().unwrap();
        let expected = [
            " m1:                      M2* ", // created a message with empty fields
            "   i2: 0               -int32 ",
            "   i3: 0               -int32 "];
        assert_eq!(app.to_strings(), expected);
    }

    #[test]
    fn delete_message_field() {
        let mut data = make_repeated_message_data(1);
        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        app.to_strings();

        app.run_command(UserCommand::ScrollVertically(1, false)).unwrap();
        app.after_event().unwrap();
        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        let expected = [
            " m1:                      M2* ",
            "   i2: 0               -int32 ", // deleted
            "   i3: 3                int32 "];
        assert_eq!(app.to_strings(), expected);
    }

    #[test]
    fn delete_message() {
        let mut data = make_repeated_message_data(2);
        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();

        app.to_strings();
        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        let expected = [
            " m1:                      M2* ", // only one message remains
            "   i2: 4                int32 ",
            "   i3: 5                int32 "];
        assert_eq!(app.to_strings(), expected);
    }

    #[test]
    fn collapse_empty_message() {
        let mut data = make_repeated_message_data(0);
        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();

        app.to_strings();
        app.run_command(UserCommand::CollapsedToggle).unwrap();
        //        app.after_event().unwrap();
        //        assert_eq!(app.to_strings(), [" m1: ... 0               -M2* "]);
        //
        //        app.run_command(UserCommand::CollapsedToggle).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), [" m1:                     -M2* "]);
    }


    // single line string displayed within apostrophes to show trailing spaces
    #[test]
    fn single_line_string() {
        let data = make_one_field_data("message M { string f1=1; }", STR("abc".to_string()));
        let mut app = App::for_tests(data, FieldOrder::Proto, 50, 25).unwrap();
        let expected = [" f1: 'abc'                                 string "];
        assert_eq!(app.to_strings(), expected);
    }

    // multiline string displayed without apostrophe
    #[test]
    fn multiline_string() {
        let data = make_one_field_data(
            "message M { string f1=1; }",
            STR("abc\ndef".to_string()));

        let mut app = App::for_tests(data, FieldOrder::Proto, 50, 25).unwrap();
        let expected = [
            " f1: abc                                   string ",
            "  2: def                                          "];
        assert_eq!(app.to_strings(), expected);
    }

    #[test]
    fn long_strings() {
        {
            let data = make_one_field_data(
                "message M { string s1=1; }",
                STR("abcdefghijklmnopqrstuvwxyz".to_string()));

            let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
            let expected = [
                " s1: abcdefghijklmnopq string ",
                "   : rstuvwxyz                "];
            assert_eq!(app.to_strings(), expected);
        }
        {
            let data = make_one_field_data(
                "message M { string s1=1; }",
                STR("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ".to_string()));

            let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
            let expected = [
                " s1: abcdefghijklmnopq string ",
                "   : rstuvwxyzABCDEFGHIJKLMNO ",
                "   : PQRSTUVWXYZ              "];
            assert_eq!(app.to_strings(), expected);
        }
        {
            let data = make_one_field_data(
                "message M { string s1=1; }",
                STR("abcdefghijklmnopqrstuvwxyz\nABCDEFGHIJKLMNOPQRSTUVWXYZ".to_string()));

            let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
            let expected = [
                " s1: abcdefghijklmnopq string ",
                "   : rstuvwxyz                ",
                "  2: ABCDEFGHIJKLMNOPQRSTUVWX ",
                "   : YZ                       "];
            assert_eq!(app.to_strings(), expected);
            //    data.add_field(&[(2, 0).into(), (6, 0).into()]).unwrap().value = FieldValue::SCALAR(STR("Leonardo's Life and Times\nLeonardo was, first of all, a painter and an artist.\nBut he was also a great thinker.".to_string()));
        }
    }

    #[test]
    fn scroll_multiline_string() {
        let data = make_one_field_data(
            "message M { string f1=1; }",
            STR("11\n22\n33\n44\n55\n66\n77\n88\n99".to_string()));

        let mut app = App::for_tests(data, FieldOrder::Proto, 20, 2 + TOP_LINE).unwrap();
        let expected0 = [
            " f1: 11      string ",
            "  2: 22             "];
        let expected1 = [
            "  2: 22             ",
            "  3: 33             "];
        assert_eq!(app.to_strings(), expected0);

        app.run_command(ScrollVertically(1, false)).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), expected0);

        app.run_command(ScrollVertically(1, false)).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), expected1);

        app.run_command(ScrollVertically(1, true)).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), expected1);

        app.run_command(ScrollVertically(1, true)).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), expected0);
    }

    #[test]
    fn repeated_strings() {
        let binary_input = [
            0x0A, 0x03, 0x61, 0x62, 0x63,
            0x0A, 0x03, 0x64, 0x65, 0x66];
        let proto = ProtoData::new("message M { repeated string f1=1; }").unwrap().finalize().unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.auto_detect_root_message().unwrap();
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        let mut app = App::for_tests(data, FieldOrder::Proto, 50, 25).unwrap();
        let expected = [
            " f1: 'abc'                                string* ",
            " f1: 'def'                                string* "
        ];
        assert_eq!(app.to_strings(), expected);
    }

    #[test]
    fn repeated_bytes() {
        let binary_input = [
            0x0A, 0x02, 0x01, 0x02,
            0x0A, 0x03, 0x03, 0x04, 0x05];
        let proto = ProtoData::new("message M { repeated bytes f1=1; }").unwrap().finalize().unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.auto_detect_root_message().unwrap();
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        let mut app = App::for_tests(data, FieldOrder::Proto, 50, 25).unwrap();
        let expected = [
            " f1: 01 02                                 bytes* ",
            " f1: 03 04 05                              bytes* "
        ];
        assert_eq!(app.to_strings(), expected);
    }


    #[test]
    fn fit_bytes_width() {
        {
            let data = make_one_field_data("message M { bytes f1=1; }", BYTES(vec![0; 16]));
            let mut app = App::for_tests(data, FieldOrder::Proto, 60, 25).unwrap();
            let expected = [" f1: 00 00 00 00 00 00 00 00  00 00 00 00 00 00 00 00 bytes "];
            assert_eq!(app.to_strings(), expected);
        }
        { // all the same but repeated field add '*'
            let data = make_one_field_data("message M { repeated bytes f1=1; }", BYTES(vec![0; 16]));
            let mut app = App::for_tests(data, FieldOrder::Proto, 60, 25).unwrap();
            let expected = [
                " f1: 00 00 00 00 00 00 00 00                         bytes* ",
                "  8: 00 00 00 00 00 00 00 00                                "];
            assert_eq!(app.to_strings(), expected);
        }
        {
            let data = make_one_field_data("message M { bytes f1=1; }", BYTES(vec![0; 16]));
            let mut app = App::for_tests(data, FieldOrder::Proto, 59, 25).unwrap();
            let expected = [
                " f1: 00 00 00 00 00 00 00 00                         bytes ",
                "  8: 00 00 00 00 00 00 00 00                               "];
            assert_eq!(app.to_strings(), expected);
        }
        {
            let data = make_one_field_data("message M { bytes f1=1; }", BYTES(vec![0; 9]));
            let mut app = App::for_tests(data, FieldOrder::Proto, 39, 25).unwrap();
            let expected = [
                " f1: 00 00 00 00 00 00 00 00  00 bytes ",
            ];
            assert_eq!(app.to_strings(), expected);
        }
        {
            let data = make_one_field_data("message M { bytes f1=1; }", BYTES(vec![0; 9]));
            let mut app = App::for_tests(data, FieldOrder::Proto, 32, 25).unwrap();
            let expected = [
                " f1: 00 00 00 00 00 00 00 bytes ",
                "  7: 00 00                      ",
            ];
            assert_eq!(app.to_strings(), expected);
        }
        {
            let data = make_one_field_data("message M { bytes f1=1; }", BYTES(vec![0; 9]));
            let mut app = App::for_tests(data, FieldOrder::Proto, 33, 25).unwrap();
            let expected = [
                " f1: 00 00 00 00 00 00 00  bytes ",
                "  7: 00 00                       ",
            ];
            assert_eq!(app.to_strings(), expected);
        }
    }

    //    #[test]
    //    fn trim_bytes_width() {
    //        {
    //            let data = make_one_field_data("message M { bytes f1=1; }", BYTES(vec![0; 16]));
    //            let mut app = App::for_tests(data, FieldOrder::Proto, 25, 25).unwrap();
    //            assert_eq!(app.to_strings()[0], " f1: 00 00 00 00 00bytes ");
    //        }
    //    }

    #[test]
    fn delete_byte() {
        {
            let bytes = (1..=8).into_iter().collect::<Vec<u8>>();
            let data = make_one_field_data("message M { bytes f1=1; }", BYTES(bytes));
            let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
            app.to_strings();
            app.run_command(UserCommand::ScrollVertically(1, false)).unwrap();
            app.after_event().unwrap();
            app.to_strings();
            app.run_command(UserCommand::DeleteData).unwrap();
            app.after_event().unwrap();
            let expected = [
                " f1: 01 02 03 04 05 06  bytes ", // data left unchanged if address row was selected
                "  6: 07 08                    "];
            assert_eq!(app.to_strings(), expected);
        }

        {
            let bytes = (1..=8).into_iter().collect::<Vec<u8>>();
            let data = make_one_field_data("message M { bytes f1=1; }", BYTES(bytes));
            let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
            let expected = [
                " f1: 01 02 03 04 05 06  bytes ",
                "  6: 07 08                    "];
            assert_eq!(app.to_strings(), expected);
            app.run_command(UserCommand::ScrollHorizontally(1)).unwrap();
            app.after_event().unwrap();
            app.to_strings();
            app.run_command(UserCommand::DeleteData).unwrap();
            app.after_event().unwrap();
            let expected = [
                " f1: 02 03 04 05 06 07  bytes ",
                "  6: 08                       "];
            assert_eq!(app.to_strings(), expected);
        }
        {
            let bytes = (1..=8).into_iter().collect::<Vec<u8>>();
            let data = make_one_field_data("message M { bytes f1=1; }", BYTES(bytes));
            let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
            app.to_strings();
            app.run_command(UserCommand::ScrollHorizontally(2)).unwrap();
            app.after_event().unwrap();
            app.to_strings();
            app.run_command(UserCommand::DeleteData).unwrap();
            app.after_event().unwrap();
            let expected = [
                " f1: 01 03 04 05 06 07  bytes ",
                "  6: 08                       "];
            assert_eq!(app.to_strings(), expected);
        }
        {
            let bytes = (1..=8).into_iter().collect::<Vec<u8>>();
            let data = make_one_field_data("message M { bytes f1=1; }", BYTES(bytes));
            let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
            app.to_strings();
            app.run_command(UserCommand::ScrollHorizontally(22)).unwrap();
            app.after_event().unwrap();
            app.to_strings();
            app.run_command(UserCommand::DeleteData).unwrap();
            app.after_event().unwrap();
            let expected = [
                " f1: 01 02 03 04 05 07  bytes ",
                "  6: 08                       "];
            assert_eq!(app.to_strings(), expected);

            app.run_command(UserCommand::DeleteData).unwrap();
            app.after_event().unwrap();
            let expected = [
                " f1: 01 02 03 04 05 08  bytes "];
            assert_eq!(app.to_strings(), expected);

            app.run_command(UserCommand::DeleteData).unwrap();
            app.after_event().unwrap();
            let expected = [
                " f1: 01 02 03 04 05     bytes "];
            assert_eq!(app.to_strings(), expected);
        }
        {
            let bytes = (1..=8).into_iter().collect::<Vec<u8>>();
            let data = make_one_field_data("message M { bytes f1=1; }", BYTES(bytes));
            let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
            app.to_strings();
            app.run_command(UserCommand::ScrollHorizontally(1)).unwrap();
            app.after_event().unwrap();
            app.to_strings();
            app.run_command(UserCommand::ScrollVertically(1, false)).unwrap();
            app.after_event().unwrap();
            app.to_strings();
            app.run_command(UserCommand::DeleteData).unwrap();
            app.after_event().unwrap();
            let expected = [
                " f1: 01 02 03 04 05 06  bytes ",
                "  6: 08                       "];
            assert_eq!(app.to_strings(), expected);

            app.run_command(UserCommand::DeleteData).unwrap();
            app.after_event().unwrap();
            let expected = [
                " f1: 01 02 03 04 05 06  bytes "];
            assert_eq!(app.to_strings(), expected);

            app.run_command(UserCommand::DeleteData).unwrap();
            app.after_event().unwrap();
            let expected = [
                " f1: 01 02 03 04 05     bytes "];
            assert_eq!(app.to_strings(), expected);
        }
        {
            let bytes = (1..=3).into_iter().collect::<Vec<u8>>();
            let data = make_one_field_data("message M { bytes f1=1; }", BYTES(bytes));
            let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
            assert_eq!(app.to_strings(), [" f1: 01 02 03           bytes "]);
            app.run_command(UserCommand::ScrollHorizontally(1)).unwrap();
            app.after_event().unwrap();
            app.to_strings();
            app.run_command(UserCommand::DeleteData).unwrap();
            app.after_event().unwrap();
            assert_eq!(app.to_strings(), [" f1: 02 03              bytes "]);

            app.run_command(UserCommand::DeleteData).unwrap();
            app.after_event().unwrap();
            assert_eq!(app.to_strings(), [" f1: 03                 bytes "]);

            app.run_command(UserCommand::DeleteData).unwrap();
            app.after_event().unwrap();
            assert_eq!(app.to_strings(), [" f1:                    bytes "]);

            app.run_command(UserCommand::DeleteData).unwrap();
            app.after_event().unwrap();
            assert_eq!(app.to_strings(), [" f1:                   -bytes "]);
        }
    }


    #[test]
    fn collapse_scalar() { // scalar layouts is not collapsable
        let data = make_test_data_1();
        let mut app = App::for_tests(data, FieldOrder::Proto, 50, 25).unwrap();

        app.to_strings();
        app.run_command(UserCommand::ScrollVertically(1, false)).unwrap();
        app.after_event().unwrap();

        app.run_command(UserCommand::CollapsedToggle).unwrap();
        app.after_event().unwrap();

        let expected = [
            " f1: 1                                      int32 ",
            " f2: 20 21                                 int32* ",
            " m3:                                           M3 ",
            "   f5: 5                                    int32 ",
            "   m6:                                        M6* ",
            "     f8: 8                                  int32 ",
            "     f9: 9                                  int32 ",
            "   m6:                                        M6* ",
            "     f8: 10                                 int32 ",
            "     f9: 11                                 int32 ",
            "   f7: 7                                    int32 ",
            " f4: 0                                     -int32 "];
        assert_eq!(app.to_strings(), expected);
    }

    #[test]
    fn collapse_expand_message() {
        let data = make_test_data_1();
        let mut app = App::for_tests(data, FieldOrder::Proto, 50, 25).unwrap();

        app.to_strings();
        app.run_command(UserCommand::ScrollVertically(2, false)).unwrap();
        app.after_event().unwrap();


        app.run_command(UserCommand::CollapsedToggle).unwrap();
        app.after_event().unwrap();

        let expected = [
            " f1: 1                                      int32 ",
            " f2: 20 21                                 int32* ",
            " m3: ... 14                                    M3 ",
            " f4: 0                                     -int32 "];
        assert_eq!(app.to_strings(), expected);

        app.run_command(UserCommand::CollapsedToggle).unwrap();
        app.after_event().unwrap();

        let expected = [
            " f1: 1                                      int32 ",
            " f2: 20 21                                 int32* ",
            " m3:                                           M3 ",
            "   f5: 5                                    int32 ",
            "   m6:                                        M6* ",
            "     f8: 8                                  int32 ",
            "     f9: 9                                  int32 ",
            "   m6:                                        M6* ",
            "     f8: 10                                 int32 ",
            "     f9: 11                                 int32 ",
            "   f7: 7                                    int32 ",
            " f4: 0                                     -int32 "];
        assert_eq!(app.to_strings(), expected);
    }


    #[test]
    fn delete_in_proto_order() {
        let binary_input = [0x08, 0x01, 0x10, 0x02, 0x18, 0x03];
        let proto = ProtoData::new("message M { int32 f1=1; int32 f2=2; int32 f3=3; }").unwrap().finalize().unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.auto_detect_root_message().unwrap();
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();


        let mut app = App::for_tests(data, FieldOrder::Proto, 50, 25).unwrap();
        let expected = [
            " f1: 1                                      int32 ",
            " f2: 2                                      int32 ",
            " f3: 3                                      int32 "];
        assert_eq!(app.to_strings(), expected);

        app.run_command(UserCommand::ScrollVertically(1, false)).unwrap();
        app.after_event().unwrap();
        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        let expected = [
            " f1: 1                                      int32 ",
            " f2: 0                                     -int32 ",
            " f3: 3                                      int32 "];
        assert_eq!(app.to_strings(), expected);

        app.run_command(UserCommand::ScrollVertically(1, false)).unwrap();
        app.after_event().unwrap();
        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        let expected = [
            " f1: 1                                      int32 ",
            " f2: 0                                     -int32 ",
            " f3: 0                                     -int32 "];
        assert_eq!(app.to_strings(), expected);
    }


    #[test]
    fn delete_in_wire_order() {
        let binary_input = [0x08, 0x01, 0x10, 0x02, 0x18, 0x03];
        let proto = ProtoData::new("message M { int32 f1=1; int32 f2=2; int32 f3=3; }").unwrap().finalize().unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.auto_detect_root_message().unwrap();
        let mut read = PbReader::new(binary_input.as_slice());
        let data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        let mut app = App::for_tests(data, FieldOrder::Wire, 30, 25).unwrap();
        let expected = [
            " f1: 1                  int32 ",
            " f2: 2                  int32 ",
            " f3: 3                  int32 "];
        assert_eq!(app.to_strings(), expected);

        app.run_command(UserCommand::ScrollVertically(1, false)).unwrap();
        app.after_event().unwrap();
        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        let expected = [
            " f1: 1                  int32 ",
            " f3: 3                  int32 "];
        assert_eq!(app.to_strings(), expected);

        app.run_command(UserCommand::ScrollVertically(1, false)).unwrap();
        app.after_event().unwrap();
        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        let expected = [
            " f1: 1                  int32 "];
        assert_eq!(app.to_strings(), expected);

        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        let expected: [&str; 0] = [];
        assert_eq!(app.to_strings(), expected);

        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        let expected: [&str; 0] = [];
        assert_eq!(app.to_strings(), expected);
    }

    #[test]
    fn delete_string() {
        let data = make_one_field_data("message M { string f1=1; }", STR("abc".to_string()));
        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        app.to_strings();
        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), [" f1: ''               -string "]);
    }

    #[test]
    fn delete_bytes() {
        let data = make_one_field_data("message M { bytes f1=1; }", BYTES([].to_vec()));
        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        app.to_strings();
        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), [" f1:                   -bytes "]);
    }

    #[test]
    fn repeated_int() {
        let mut data = make_one_field_data("message M { repeated int32 i1=1; }", I32(1));

        data.add_field(&[(1, 1).into()]).unwrap().value = FieldValue::SCALAR(I32(2));
        data.add_field(&[(1, 2).into()]).unwrap().value = FieldValue::SCALAR(I32(3));

        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        assert_eq!(app.to_strings(), [" i1: 1 2 3             int32* "]);

        //app.run_command(UserCommand::DeleteData).unwrap();
        //app.after_event().unwrap();
    }

    fn make_repeated_int_data() -> App {
        let mut data = make_no_field_data("message M { repeated int32 i1=1; }");
        for v in 1..=6 {
            data.add_field(&[(1, usize::MAX).into()]).unwrap().value = SCALAR(I32(v));
        }

        let mut app = App::for_tests(data, FieldOrder::Proto, 20, 25).unwrap();
        assert_eq!(app.to_strings(), [" i1: 1 2 3 4 int32* ", "  4: 5 6            "]);
        app
    }

    #[test]
    fn delete_repeated_int() {
        let mut app = make_repeated_int_data();

        app.run_command(UserCommand::ScrollVertically(1, false)).unwrap();
        app.after_event().unwrap();
        app.run_command(UserCommand::ScrollHorizontally(1)).unwrap();
        app.after_event().unwrap();

        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), [
            " i1: 1 2 3 4 int32* ",
            "  4: 6              "]);

        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), [
            " i1: 1 2 3 4 int32* "]);

        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), [
            " i1: 1 2 3   int32* "]);

        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), [
            " i1: 1 2     int32* "]);

        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), [
            " i1: 1       int32* "]);

        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), [
            " i1: 0      -int32* "]);

        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), [
            " i1: 0      -int32* "]);
    }


    #[test]
    fn insert_int() {
        fn test_fn(scroll_x: usize, scroll_y: usize, expected: Vec<&str>) {
            let mut app = make_repeated_int_data();

            for _ in 0..scroll_y {
                app.run_command(UserCommand::ScrollVertically(1, false)).unwrap();
                app.after_event().unwrap();
            }
            for _ in 0..scroll_x {
                app.run_command(UserCommand::ScrollHorizontally(1)).unwrap();
                app.after_event().unwrap();
            }

            app.run_command(UserCommand::InsertData).unwrap();
            app.after_event().unwrap();
            assert_eq!(app.to_strings(), expected);
        }

        let expected = [
            " i1: 0 1 2 3 int32* ",
            "  4: 4 5 6          "].to_vec();
        test_fn(0, 0, expected);

        let expected = [
            " i1: 1 0 2 3 int32* ",
            "  4: 4 5 6          "].to_vec();
        test_fn(1, 0, expected);

        let expected = [
            " i1: 1 2 0 3 int32* ",
            "  4: 4 5 6          "].to_vec();
        test_fn(2, 0, expected);

        let expected = [
            " i1: 1 2 3 4 int32* ",
            "  4: 5 0 6          "].to_vec();
        test_fn(1, 1, expected);

        let expected = [
            " i1: 1 2 3 4 int32* ",
            "  4: 5 6 0          "].to_vec();
        test_fn(5, 1, expected);
    }

    //    #[test]
    //    fn insert_int() {
    //        let mut data = make_no_field_data("message M { repeated int32 i1=1; }");
    //
    //        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
    //        assert_eq!(app.to_strings(), [" i1: 0                -int32* "]);
    //
    //        app.run_command(UserCommand::InsertData).unwrap();
    //        app.after_event().unwrap();
    //        assert_eq!(app.to_strings(), [" i1: 0                 int32* "]);
    //
    //        app.run_command(UserCommand::InsertData).unwrap();
    //        app.after_event().unwrap();
    //        assert_eq!(app.to_strings(), [" i1: 0 0               int32* "]);
    //    }


    #[test]
    fn repeated_multiline_int() {
        fn test_data() -> MessageData {
            let mut data = make_one_field_data("message M { repeated int32 i1=1; }", I32(2));
            for v in 2..10 {
                data.add_field(&[(1, usize::MAX).into()]).unwrap().value = FieldValue::SCALAR(I32(v * 2));
            }
            data
        }
        {
            let mut app = App::for_tests(test_data(), FieldOrder::Proto, 25, 25).unwrap();
            let expected = [
                " i1: 2 4 6 8 10   int32* ",
                "  5: 12 14 16 18         "];
            assert_eq!(app.to_strings(), expected);
        }
        {
            let mut app = App::for_tests(test_data(), FieldOrder::Proto, 26, 25).unwrap();
            let expected = [
                " i1: 2 4 6 8 10 12 int32* ",
                "  6: 14 16 18             "];
            assert_eq!(app.to_strings(), expected);
        }
    }

    #[test]
    fn nested_repeated_strings() {
        let proto_str = "message M { M2 m2 = 2; }\nmessage M2 { repeated string s1 = 1; }";
        let binary_input = [
            0x12, 8,  //              m2: M2
            0x0A, 2, 0x77, 0x77, //     s1: 2 "ww"
            0x0A, 2, 0x78, 0x78, //     s1: 2 "xx"
        ];

        let proto = ProtoData::new(proto_str).unwrap().finalize().unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.auto_detect_root_message().unwrap();
        let mut read = PbReader::new(binary_input.as_slice());
        let mut data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        let expected = [
            " m2:                       M2 ",
            "   s1: 'ww'           string* ",
            "   s1: 'xx'           string* "];
        assert_eq!(app.to_strings(), expected);
    }

    #[test]
    fn nested_repeated_int() {
        let proto_str = "message M { M2 m2 = 2; }\nmessage M2 { repeated int32 i1 = 1; }";
        let binary_input = [
            0x12, 4, // m2: M2
            0x08, 1, //   i1: 1
            0x08, 2, //   i1: 2
        ];

        let proto = ProtoData::new(proto_str).unwrap().finalize().unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.auto_detect_root_message().unwrap();
        let mut read = PbReader::new(binary_input.as_slice());
        let mut data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        let expected = [
            " m2:                       M2 ",
            "   i1: 1 2             int32* "];
        assert_eq!(app.to_strings(), expected);
    }


    #[test]
    fn nested_long_name() {
        let proto_str = "message M { M2 m2 = 2; }\nmessage M2 { M3 m3 = 2; int32 longname = 3; }\nmessage M3 { M4 m4 = 2; }\nmessage M4 { int32 i = 1; }";
        let binary_input = [
            0x12, 4, // m2: M2
            0x12, 2, // m3: M3
            0x12, 0, // m4: M4
        ];

        let proto = ProtoData::new(proto_str).unwrap().finalize().unwrap();
        let mut limit = binary_input.len() as u32;
        let root_msg = proto.auto_detect_root_message().unwrap();
        let mut read = PbReader::new(binary_input.as_slice());
        let mut data = MessageData::new(&mut read, &proto, root_msg, &mut limit).unwrap();

        //println!("{:?}", proto);

        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();

        println!("{:?}", app.layouts.indents);

        let expected = [
            " m2:                       M2 ",
            "       m3:                 M3 ",
            "         m4:               M4 ",
            "            i: 0       -int32 ",
            " longname: 0           -int32 "];
        assert_eq!(app.to_strings(), expected);
    }

    #[test]
    fn empty_string() {
        let mut data = make_no_field_data("message M {  string s1=1; }");
        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        assert_eq!(app.to_strings(), [" s1: ''               -string "]);
    }


    #[test]
    fn insert_string() {
        let mut data = make_one_field_data("message M { repeated string s1=1; }", STR("1".to_string()));
        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        app.to_strings();
        app.run_command(UserCommand::InsertData).unwrap();
        app.after_event().unwrap();
        let expected = [
            " s1: ''               string* ", // default value inserted
            " s1: '1'              string* "];
        assert_eq!(app.to_strings(), expected);
    }


    #[test]
    fn delete_repeated_string() {
        let mut data = make_one_field_data("message M { repeated string s1=1; }", STR("1".to_string()));
        data.add_field(&[(1, 1).into()]).unwrap().value = FieldValue::SCALAR(STR("2".to_string()));

        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        let expected = [
            " s1: '1'              string* ",
            " s1: '2'              string* "];
        assert_eq!(app.to_strings(), expected);

        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), [" s1: '2'              string* "]);

        app.run_command(UserCommand::DeleteData).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), [" s1: ''              -string* "]);
    }

    #[test]
    fn empty_enum() {
        let mut data = make_no_field_data("enum E1 { V1=0; V2=1; }\nmessage M { E1 e1=1; }");
        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        assert_eq!(app.to_strings(), [" e1: V1                   -E1 "]);
    }

    #[test]
    fn repeated_enum() {
        let mut data = make_one_field_data("enum E1 { V1=0; V2=1; }\nmessage M { repeated E1 e1=1; }", ENUM(1));
        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        assert_eq!(app.to_strings(), [" e1: V2                   E1* "]);

        app.run_command(UserCommand::InsertData).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), [" e1: V1 V2                E1* "]);
    }

    #[test]
    fn empty_float() {
        let mut data = make_no_field_data("message M { float f1=1; }");
        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        assert_eq!(app.to_strings(), [" f1: 0                 -float "]);
    }

    #[test]
    fn simple_double() {
        let mut data = make_one_field_data("message M { double f1=1; }", F64(11.1));
        let mut app = App::for_tests(data, FieldOrder::Proto, 30, 25).unwrap();
        assert_eq!(app.to_strings(), [" f1: 11.1              double "]);
    }

    #[test]
    fn scroll_repeated_message() {
        let mut data = make_repeated_message_data(3);
        let mut app = App::for_tests(data, FieldOrder::Proto, 20, 3 + TOP_LINE).unwrap();

        let expected0 = [
            " m1:            M2* ",
            "   i2: 2      int32 ",
            "   i3: 3      int32 "];
        assert_eq!(app.to_strings(), expected0);

        app.run_command(ScrollVertically(1, false)).unwrap(); // next line
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), expected0);

        app.run_command(ScrollVertically(1, false)).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), expected0);

        app.run_command(ScrollVertically(1, false)).unwrap(); // scroll one line down
        app.after_event().unwrap();
        let expected1 = [
            "   i2: 2      int32 ",
            "   i3: 3      int32 ",
            " m1:            M2* "];
        assert_eq!(app.to_strings(), expected1);

        app.run_command(ScrollVertically(1, true)).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), expected1);

        app.run_command(ScrollVertically(1, true)).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), expected1);

        app.run_command(ScrollVertically(1, true)).unwrap(); // scroll one line up
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), expected0);

        app.run_command(ScrollVertically(99, false)).unwrap(); // scroll to end
        app.after_event().unwrap();
        let expected_end = [
            " m1:            M2* ",
            "   i2: 6      int32 ",
            "   i3: 7      int32 "];
        assert_eq!(app.to_strings(), expected_end);

        app.run_command(ScrollVertically(1, false)).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), expected_end);

        app.run_command(ScrollVertically(99, true)).unwrap(); // scroll to start
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), expected0);

        app.run_command(ScrollVertically(1, true)).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), expected0);
    }

    #[test]
    fn scroll_up_repeated_message() {
        let mut data = make_repeated_message_data(10);
        let mut app = App::for_tests(data, FieldOrder::Proto, 20, 3 + TOP_LINE).unwrap();

        let expected_start = [
            " m1:            M2* ",
            "   i2: 2      int32 ",
            "   i3: 3      int32 "];
        assert_eq!(app.to_strings(), expected_start);

        app.run_command(ScrollToBottom).unwrap(); // end of the file
        app.after_event().unwrap();

        let expected_end = [
            " m1:            M2* ",
            "   i2: 20     int32 ",
            "   i3: 21     int32 "];
        assert_eq!(app.to_strings(), expected_end);

        app.run_command(ScrollVertically(3, true)).unwrap(); // scroll up
        app.after_event().unwrap();

        let expected_end = [
            "   i3: 19     int32 ",
            " m1:            M2* ",
            "   i2: 20     int32 "];
        assert_eq!(app.to_strings(), expected_end);
    }

    #[test]
    fn scroll_empty() {
        let mut data = make_no_field_data("message M { string f1=1; }");
        let mut app = App::for_tests(data, FieldOrder::Wire, 30, 25).unwrap();
        app.to_strings();
        app.run_command(UserCommand::ScrollVertically(1, true)).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), Vec::<String>::new());
        app.run_command(UserCommand::ScrollVertically(1, false)).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), Vec::<String>::new());
        app.run_command(UserCommand::ScrollSibling(1)).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), Vec::<String>::new());
        app.run_command(UserCommand::ScrollSibling(-1)).unwrap();
        app.after_event().unwrap();
        assert_eq!(app.to_strings(), Vec::<String>::new());
    }


    #[test]
    fn change_field_order() {
        let mut data = make_one_field_data("message M { int32 x=2; int32 y=1; }", ScalarValue::I32(3));
        let mut app = App::for_tests(data, FieldOrder::Proto, 20, 25).unwrap();

        let expected_start = [
            " x: 0        -int32 ",
            " y: 3         int32 "];
        assert_eq!(app.to_strings(), expected_start);

        app.run_command(UserCommand::ChangeFieldOrder(FieldOrder::ById)).unwrap();
        app.after_event().unwrap();
        let expected_start = [
            " y: 3         int32 ",
            " x: 0        -int32 "
        ];
        assert_eq!(app.to_strings(), expected_start);

        app.run_command(UserCommand::ChangeFieldOrder(FieldOrder::Wire)).unwrap();
        app.after_event().unwrap();
        let expected_start = [
            " y: 3         int32 "];
        assert_eq!(app.to_strings(), expected_start);

        app.run_command(UserCommand::ChangeFieldOrder(FieldOrder::ByName)).unwrap();
        app.after_event().unwrap();
        let expected_start = [
            " x: 0        -int32 ",
            " y: 3         int32 "];
        assert_eq!(app.to_strings(), expected_start);
    }


    //    #[test]
    //    fn make_test_data_file() {
    //        let proto_str = "message M { string s1=1; }";
    //        let mut data = make_no_field_data(proto_str);
    //        let proto = ProtoData::new(proto_str).unwrap();
    //        let mut field = data.add_field(&[(1, 0).into()]).unwrap();
    //        field.value = SCALAR(STR("Leonardo's Life and Times\nLeonardo was, first of all, a painter and an artist.\nBut he was also a great thinker.".to_string()));
    //        let mut f = std::fs::File::create("str.pb").unwrap();
    //        data.write(&mut f, &proto, proto.root_message()).unwrap();

    //        let proto_str = "message M { bytes b1=1; }";
    //        let mut data = make_no_field_data(proto_str);
    //        let proto = ProtoData::new(proto_str).unwrap();
    //        let mut field = data.add_field(&[(1, 0).into()]).unwrap();
    //        let data_vec = (0..1000u64).into_iter().map(|n|((n*8902374+59036783)&0xff) as u8).collect();
    //        field.value = SCALAR(BYTES(data_vec));
    //        let mut f = std::fs::File::create("bytes.pb").unwrap();
    //        data.write(&mut f, &proto, proto.root_message()).unwrap();
    //
    //    }


    // TODO unknown field layout
    // TODO delete a field of a submesage

    // if user start typing when a scalar value selected, the old values will be discarded and replaced by the new
    // if user press Enter when a scalar value selected it starts editing of the current value
}