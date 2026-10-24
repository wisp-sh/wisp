use wisp_wisp__context_values::Library;
fn main()->Result<(),Box<dyn std::error::Error>>{
 let library=Library::new()?;let mut errors=library.errors();
 let mut context=errors.create()?;context.close();assert!(context.is_closed());
 assert!(errors.describe(context).unwrap_err().to_string().contains("closed"));
 let other=Library::new()?;let context=other.errors().create()?;
 assert!(errors.describe(context).unwrap_err().to_string().contains("another Component"));
 for _ in 1..127 {let context=errors.create()?;assert_eq!(errors.describe(context)?,"fixture error");}
 assert!(errors.create().unwrap_err().to_string().contains("capacity"));
 drop(errors);drop(library);
 let fresh=Library::new()?;let mut errors=fresh.errors();let context=errors.create()?;
 assert_eq!(errors.describe(context)?,"fixture error");
 println!("installed error contexts: typed message, closed, cross-instance, Store limit passed");Ok(())
}
