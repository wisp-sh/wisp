wit_bindgen::generate!({path: "wit", world: "provider"});
struct Provider;
impl exports::demo::a::api::Guest for Provider {
    fn echo(value: exports::demo::a::types::Payload) -> exports::demo::a::types::Payload { value }
}
export!(Provider);
