mod wisp_bindings;
use wisp_bindings::exports::demo::rust_values::api;
struct Values;
impl api::Guest for Values {
    fn echo(value: String) -> String { value }
    fn buffer(value: Vec<u8>) -> Vec<u8> { value }
    fn packet_value(value: api::Packet) -> api::Packet { value }
    fn wide_value(value: api::Wide) -> api::Wide { value }
    fn note(value: api::WispNote) -> api::WispNote { value }
    fn labels(value: Vec<String>) -> Vec<String> { value }
    fn selected(value: Option<api::Mode>) -> Option<api::Mode> { value }
    fn modes(value: Vec<api::Mode>) -> Vec<api::Mode> { value }
    fn choices(value: Vec<Option<api::Mode>>) -> Vec<Option<api::Mode>> { value }
    fn batch_value(value: api::Batch) -> api::Batch { value }
    fn result_value(value: Result<api::Packet, String>) -> Result<api::Packet, String> { value }
    fn checked(success: bool) -> Result<api::Packet, String> {
        if !success { return Err("denied".into()); }
        Ok(api::Packet { label: "hello 🦀".into(), data: vec![0,128,255], sequence: 9007199254740994, enabled: true })
    }
}
wisp_bindings::export!(Values with_types_in wisp_bindings);
