
// TODOtrait ScreenItem {
// TODO    fn title(&self) -> String;
// TODO    fn tag(&self) -> Option<TLV>;
// TODO    fn content(&self, width: u16) -> String;
// TODO    fn typename(&self) -> String;
// TODO    fn height(&self, width: u16) -> (u32, u32);
// TODO}
// TODO
// TODO//enum ScreenItem {
// TODO//    Scalar(),
// TODO//    Message(),
// TODO//    Array(),
// TODO//    Table(),
// TODO//}
// TODO
// TODOstruct DataScreen {
// TODO    first_line: FieldPath,
// TODO    lines: Vec<Box<dyn ScreenItem>>,
// TODO}