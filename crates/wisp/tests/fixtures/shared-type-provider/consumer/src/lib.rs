wit_bindgen::generate!({path: "wit", world: "consumer", generate_all});
struct Consumer;
impl exports::demo::b::api::Guest for Consumer {
    fn echo(value: demo::a::types::Payload) -> demo::a::types::Payload { demo::a::api::echo(value) }
}
export!(Consumer);
