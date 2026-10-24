wit_bindgen::generate!({path:"wit",world:"consumer",with:{"wisp:context-values/errors@0.1.0":wisp_wisp__context_values::errors}});
struct Consumer;
impl exports::wisp::context_consumer::check::Guest for Consumer {
 fn run()->u32 {let library=wisp_wisp__context_values::Library::new().unwrap();let mut errors=library.errors();
 for _ in 0..8 {let context=errors.create().unwrap();assert_eq!(errors.describe(context).unwrap(),"fixture error");}
 let mut context=errors.create().unwrap();context.close();assert!(context.is_closed());42}
}
export!(Consumer);
