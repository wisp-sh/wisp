use std::cell::RefCell;
use wasi::http::{outgoing_handler, types::{Fields, Method, OutgoingRequest, Scheme}};
use wasi::io::streams::StreamError;
use wasi::sockets::{instance_network, ip_name_lookup, network::{IpAddressFamily, IpSocketAddress, Ipv4SocketAddress}, tcp::ShutdownType, tcp_create_socket};
wit_bindgen::generate!({ path: "wit", world: "library", generate_all });
use exports::wisp::network::requests::{Guest, GuestResponse, Response};
struct Component;
struct Reply { status: u16, body: RefCell<String> }
impl GuestResponse for Reply {
    fn status(&self) -> u16 { self.status }
    fn body(&self) -> String { self.body.borrow().clone() }
}
impl Guest for Component {
    type Response = Reply;
    fn fetch(url: String) -> Result<Response, String> {
        let parsed = url.strip_prefix("http://").ok_or("expected http URL")?;
        let (authority, path) = parsed.split_once('/').unwrap_or((parsed, ""));
        let request = OutgoingRequest::new(Fields::new());
        request.set_method(&Method::Get).map_err(|_| "method")?;
        request.set_scheme(Some(&Scheme::Http)).map_err(|_| "scheme")?;
        request.set_authority(Some(authority)).map_err(|_| "authority")?;
        request.set_path_with_query(Some(&format!("/{path}"))).map_err(|_| "path")?;
        let future = outgoing_handler::handle(request, None).map_err(|e| format!("{e:?}"))?;
        future.subscribe().block();
        let response = future.get().ok_or("pending")?.map_err(|_| "future failed")?.map_err(|e| format!("{e:?}"))?;
        let status = response.status();
        let body = response.consume().map_err(|_| "body")?;
        let stream = body.stream().map_err(|_| "stream")?;
        let mut bytes = Vec::new();
        loop {
            match stream.blocking_read(4096) {
                Ok(chunk) => { bytes.extend_from_slice(&chunk); if bytes.len() > 65536 { return Err("body size limit".into()) } }
                Err(StreamError::Closed) => break,
                Err(error) => return Err(format!("{error:?}")),
            }
        }
        drop(stream);
        drop(body);
        drop(response);
        drop(future);
        Ok(Response::new(Reply { status, body: RefCell::new(String::from_utf8(bytes).map_err(|_| "utf8")?) }))
    }
    fn resolve(name: String) -> Result<u32, String> {
        let network = instance_network::instance_network();
        let stream = ip_name_lookup::resolve_addresses(&network, &name).map_err(|e| format!("{e:?}"))?;
        let mut count = 0;
        loop {
            stream.subscribe().block();
            match stream.resolve_next_address().map_err(|e| format!("{e:?}"))? {
                Some(_) => count += 1,
                None => return Ok(count),
            }
        }
    }
    fn connect(port: u16) -> Result<(), String> {
        let network = instance_network::instance_network();
        let socket = tcp_create_socket::create_tcp_socket(IpAddressFamily::Ipv4).map_err(|e| format!("{e:?}"))?;
        socket.start_connect(&network, IpSocketAddress::Ipv4(Ipv4SocketAddress { port, address: (127, 0, 0, 1) })).map_err(|e| format!("{e:?}"))?;
        socket.subscribe().block();
        let (input, output) = socket.finish_connect().map_err(|e| format!("{e:?}"))?;
        socket.shutdown(ShutdownType::Both).map_err(|e| format!("{e:?}"))?;
        drop(input); drop(output); drop(socket);
        Ok(())
    }
}
export!(Component);
