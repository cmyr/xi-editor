// Copyright 2016 Google Inc. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! RPC handling for communications with front-end.

use serde_json::{self, Value};
use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{self, Serialize, Serializer};

use tabs::ViewIdentifier;
use plugins::PlaceholderRpc;


// =============================================================================
//  Command types
// =============================================================================

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct EmptyStruct {}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
pub enum CoreNotification {
    Edit(EditCommand<EditNotification>),
    Plugin(PluginNotification),
    CloseView { view_id: ViewIdentifier },
    Save { view_id: ViewIdentifier, file_path: String },
    SetTheme { theme_name: String },
    ClientStarted(EmptyStruct),
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
pub enum CoreRequest {
    Edit(EditCommand<EditRequest>),
    NewView { file_path: Option<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditCommand<T> {
    pub view_id: ViewIdentifier,
    pub cmd: T,
}

/// An enum representing touch and mouse gestures applied to the text.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum GestureType {
    ToggleSel,
}

// NOTE:
// Several core protocol commands use a params array to pass arguments
// which are named, internally. these two types use custom Serialize /
// Deserialize impls to accomodate this.
#[derive(PartialEq, Eq, Debug)]
pub struct LineRange {
    pub first: i64,
    pub last: i64,
}

#[derive(PartialEq, Eq, Debug)]
pub struct MouseAction {
    pub line: u64,
    pub column: u64,
    pub flags: u64,
    pub click_count: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
pub enum EditNotification {
    Insert { chars: String },
    DeleteForward,
    DeleteBackward,
    DeleteWordForward,
    DeleteWordBackward,
    DeleteToEndOfParagraph,
    DeleteToBeginningOfLine,
    InsertNewline,
    InsertTab,
    MoveUp,
    MoveUpAndModifySelection,
    MoveDown,
    MoveDownAndModifySelection,
    MoveLeft,
    // synoynm for `MoveLeft`
    MoveBackward,
    MoveLeftAndModifySelection,
    MoveRight,
    // synoynm for `MoveRight`
    MoveForward,
    MoveRightAndModifySelection,
    MoveWordLeft,
    MoveWordLeftAndModifySelection,
    MoveWordRight,
    MoveWordRightAndModifySelection,
    MoveToBeginningOfParagraph,
    MoveToEndOfParagraph,
    MoveToLeftEndOfLine,
    MoveToLeftEndOfLineAndModifySelection,
    MoveToRightEndOfLine,
    MoveToRightEndOfLineAndModifySelection,
    MoveToBeginningOfDocument,
    MoveToBeginningOfDocumentAndModifySelection,
    MoveToEndOfDocument,
    MoveToEndOfDocumentAndModifySelection,
    ScrollPageUp,
    PageUpAndModifySelection,
    ScrollPageDown,
    PageDownAndModifySelection,
    SelectAll,
    AddSelectionAbove,
    AddSelectionBelow,
    Scroll(LineRange),
    GotoLine { line: u64 },
    RequestLines(LineRange),
    Yank,
    Transpose,
    Click(MouseAction),
    Drag(MouseAction),
    Gesture { line: u64, column: u64, ty: GestureType},
    Undo,
    Redo,
    FindNext { wrap_around: Option<bool>, allow_same: Option<bool> },
    FindPrevious { wrap_around: Option<bool> },
    DebugRewrap,
    DebugPrintSpans,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
pub enum EditRequest {
    Cut,
    Copy,
    Find { chars: Option<String>, case_sensitive: bool },
}


#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "command")]
#[serde(rename_all = "snake_case")]
pub enum PluginNotification {
    Start { view_id: ViewIdentifier, plugin_name: String },
    Stop { view_id: ViewIdentifier, plugin_name: String },
    PluginRpc { view_id: ViewIdentifier, receiver: String, rpc: PlaceholderRpc },
}

// Serialize / Deserialize

impl<T: Serialize> Serialize for EditCommand<T>
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer
    {
        let mut v = serde_json::to_value(&self.cmd).map_err(ser::Error::custom)?;
        v["params"]["view_id"] = json!(self.view_id);
        v.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for EditCommand<T>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de>
    {
        #[derive(Deserialize)]
        struct InnerId {
            view_id: ViewIdentifier,
        }

        let mut v = Value::deserialize(deserializer)?;
        let helper = InnerId::deserialize(&v).map_err(de::Error::custom)?;
        let InnerId { view_id } = helper;
        // if params are empty, remove them
        let remove_params = match v.get("params") {
            Some(&Value::Object(ref obj)) => obj.is_empty(),
            Some(&Value::Array(ref arr)) => arr.is_empty(),
            Some(_) => return Err(de::Error::custom("'params' field, if present, must be object or array.")),
            None => false,
        };

        if remove_params {
            v.as_object_mut().map(|v| v.remove("params"));
        }

        let cmd = T::deserialize(v).map_err(de::Error::custom)?;
        Ok(EditCommand { view_id, cmd })
    }
}

impl Serialize for MouseAction
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer
    {
        #[derive(Serialize)]
        struct Helper(u64, u64, u64, Option<u64>);

        let as_tup = Helper(self.line, self.column, self.flags, self.click_count);
        let v = serde_json::to_value(&as_tup).map_err(ser::Error::custom)?;
        v.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for MouseAction
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de>
    {
        let v: Vec<u64> = Vec::deserialize(deserializer)?;
        let click_count = if v.len() == 4 { Some(v[3]) } else { None };
        Ok(MouseAction { line: v[0], column: v[1], flags: v[2], click_count: click_count })
    }
}

impl Serialize for LineRange
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer
    {
        let as_tup = (self.first, self.last);
        let v = serde_json::to_value(&as_tup).map_err(ser::Error::custom)?;
        v.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LineRange
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de>
    {
        #[derive(Deserialize)]
        struct TwoTuple(i64, i64);

        let tup = TwoTuple::deserialize(deserializer)?;
        Ok(LineRange { first: tup.0, last: tup.1 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const TEST_JSON: &str = r#"{"method":"client_started","params":{}}
{"method":"set_theme","params":{"theme_name":"InspiredGitHub"}}
{"method":"close_view","params":{"view_id":"view-id-3"}}
{"id":0,"method":"new_view","params":{}}
{"method":"save","params":{"view_id":"view-id-5","file_path":"path.rs"}}
{"id":1,"method":"new_view","params":{"file_path":"path.rs"}}
{"method":"plugin","params":{"command":"start","view_id":"view-id-5","plugin_name":"spellcheck"}}
{"method":"plugin","params":{"command":"stop","view_id":"view-id-5","plugin_name":"spellcheck"}}
{"method":"edit","params":{"view_id":"view-id-5","method":"insert","params":{"chars":"a"}}}
{"method":"edit","params":{"view_id":"view-id-5","method":"delete_backward","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"delete_forward","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"delete_word_forward","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"delete_word_backward","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"delete_to_end_of_paragraph","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"insert_newline","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"insert_tab","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_up","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_down","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_up_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_down_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_left","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_right","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_left_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_right_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_word_left","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_word_right","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_word_left_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_word_right_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_to_beginning_of_paragraph","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_to_end_of_paragraph","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_to_left_end_of_line","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_to_left_end_of_line_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_to_right_end_of_line","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_to_right_end_of_line_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_to_beginning_of_document","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_to_beginning_of_document_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_to_end_of_document","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"move_to_end_of_document_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"scroll_page_up","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"scroll_page_down","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"page_up_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"page_down_and_modify_selection","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"select_all","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"add_selection_above","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"add_selection_below","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"scroll","params":[5,57]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"goto_line","params":{"line":1}}}
{"method":"edit","params":{"view_id":"view-id-3","method":"request_lines","params":[12,52]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"transpose","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"yank","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"click","params":[6,0,0,1]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"drag","params":[17,15,0]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"undo","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"redo","params":[]}}
{"id":4,"method":"edit","params":{"view_id":"view-id-5","method":"find","params":{"case_sensitive":false,"chars":"m"}}}
{"method":"edit","params":{"view_id":"view-id-5","method":"find_next","params":{"wrap_around":true}}}
{"method":"edit","params":{"view_id":"view-id-5","method":"find_previous","params":{"wrap_around":true}}}
{"method":"edit","params":{"view_id":"view-id-5","method":"debug_rewrap","params":[]}}
{"method":"edit","params":{"view_id":"view-id-5","method":"debug_print_spans","params":[]}}
{"id":11,"method":"edit","params":{"view_id":"view-id-5","method":"cut","params":[]}}
{"id":11,"method":"edit","params":{"view_id":"view-id-5","method":"copy","params":[]}}"#;

#[test]
fn test_parse() {
    for json in TEST_JSON.lines() {
        let parsed: Value = match serde_json::from_str(json) {
            Ok(p) => p,
            Err(e) => panic!("{:?}\n{}", e, json),
        };

        let is_req = parsed.get("id").is_some();
        let err = if is_req {
            serde_json::from_value::<CoreRequest>(parsed).err()
        } else {
            serde_json::from_value::<CoreNotification>(parsed).err()
        };
        if err.is_some() {
            panic!("{:?}\n{}", err, json)
        }
    }
}
}
