#[derive(Debug)]
pub enum CallError<E> { Guest(E), Runtime(String) }
impl<E:std::fmt::Debug> std::fmt::Display for CallError<E> {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{write!(f,"{self:?}")}}
impl<E:std::fmt::Debug> std::error::Error for CallError<E>{}
struct State {client:protocol::client::Client,instance:u64,poisoned:bool}
#[derive(Clone)]
pub struct Library {state:std::sync::Arc<std::sync::Mutex<State>>}
impl Library {
    pub fn new()->Result<Self,CallError<std::convert::Infallible>> {
        let create=||->Result<Self,String>{
            use std::os::unix::fs::PermissionsExt;
            let platform=match (std::env::consts::OS,std::env::consts::ARCH) {
                ("linux","x86_64")=>"linux-amd64",("macos","aarch64")=>"darwin-arm64",_=>return Err("unsupported native worker platform".into()),
            };
            let bytes:&[u8]=match platform {@WORKER_SELECTION@ _=>return Err(format!("package has no bundled worker for {platform}"))};
            let temp=tempfile::tempdir().map_err(|e|e.to_string())?;
            let worker=temp.path().join("wisp-worker");
            std::fs::write(&worker,bytes).map_err(|e|e.to_string())?;
            std::fs::set_permissions(&worker,std::fs::Permissions::from_mode(0o700)).map_err(|e|e.to_string())?;
            let module=temp.path().join("component.wasm");
            std::fs::write(&module,include_bytes!("../component.wasm")).map_err(|e|e.to_string())?;
            let mut client=protocol::client::Client::spawn(&worker,None).map_err(|e|e.to_string())?;
            let response=client.request(protocol::Request{operation:"open".into(),module:module.to_str().ok_or("non-UTF8 temporary path")?.into(),wasi_p3:true,..Default::default()}).map_err(|e|e.to_string())?;
            if !response.ok {return Err(response.error.unwrap_or_else(||"worker initialization failed".into()));}
            Ok(Self{state:std::sync::Arc::new(std::sync::Mutex::new(State{client,instance:response.instance,poisoned:false}))})
        };
        create().map_err(CallError::Runtime)
    }
    fn call(&self,interface:&str,function:&str,params:Vec<serde_json::Value>)->Result<Vec<serde_json::Value>,String>{
        let mut state=self.state.try_lock().map_err(|_|"Component instance is busy or poisoned".to_owned())?;
        if state.poisoned {return Err("Component instance is poisoned".into());}
        let request=protocol::Request{operation:"call".into(),instance:state.instance,interface:interface.into(),function:function.into(),params,..Default::default()};
        let result=state.client.request(request).map_err(|e|e.to_string()).and_then(|r|if r.ok{Ok(r.results)}else{Err(r.error.unwrap_or_else(||"worker call failed".into()))});
        if result.is_err(){state.poisoned=true;state.client.terminate();}
        result
    }
}
