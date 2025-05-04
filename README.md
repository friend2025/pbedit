# pbedit

Òerminal-based [protobuf](https://protobuf.dev) data files editor.

## Command Line Interface

`editor data.pb;format.proto;message_name`

 * data.pb - path to file in protobuf format
 * format.proto - path to .proto file with data description
 * message_name - name of the root message in .proto (optional)

## Hotkeys

Up/Down - Navigate lines

Ctrl+Up/Down - Navigate field of a message

F4 - Change field sort order. Four variants available:

 * Proto - field shown as in the order it written in the proto file. This is default mode.
 * Wire - field shown as it readed in the binary data file. In this mode shown only data realy readed from the file (no default values).
 * Name - filed sorted by its name.
 * Id - filed sorted by numbers specified in the proto file.

 The first char of sort mode is at the end of the top line.

F5/Enter - Expand/Collapse data

F10/Esc - Quit

Del - Delete data

Ins - Insert data


## Limits

In the current version, the program may slow down with files larger than 10 thousand data items.

## Examples

There are several example data files for testing the application in the 'data' folder.
Type `editor filename.pb` to open a file.

 * ints.pb - simple integer data example
 * str.pb - multiline string example
 * bytes.pb - a field with 1000 random bytes
 * mega.pb - 1 million random values in three-level structures
 * test_data_1.pb - simple nested data example

