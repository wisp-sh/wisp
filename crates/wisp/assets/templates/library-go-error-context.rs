impl Client {
    fn decode_error_context(&mut self,value:Value)->WireResult<wasmtime::component::ErrorContext>{
        let mut fields=object(value)?;let owner=scalar::<u64>(field(&mut fields,"owner")?)?;let id=scalar::<u64>(field(&mut fields,"id")?)?;finished(fields)?;
        if owner!=self.owner{return Err("error context belongs to another Component instance".into())}
        let context=self.contexts.remove(&id).ok_or("error context is closed")?;
        self.consumed_contexts.push(id);Ok(context)
    }
    fn encode_error_context(&mut self,context:wasmtime::component::ErrorContext)->WireResult<Value>{
        if self.error_context_lifts>=127{return Err("error-context Store lifetime capacity reached (127 lifts)".into())}
        let id=self.next_context;self.next_context=self.next_context.checked_add(1).ok_or("error context identity exhausted")?;
        self.contexts.insert(id,context);self.error_context_lifts+=1;Ok(json!({"owner":self.owner,"id":id}))
    }
    fn close_context(&mut self,id:u64)->WireResult<()> {self.contexts.remove(&id);Ok(())}
}
