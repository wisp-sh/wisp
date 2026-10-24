pub struct Packet {
    pub label: String,
    pub data: Vec<u8>,
    pub sequence: u64,
    pub enabled: bool,
}

pub fn echo(value: String) -> String { value }
pub fn buffer(value: Vec<u8>) -> Vec<u8> { value }
pub fn packet(value: Packet) -> Packet { value }
pub fn checked(success: bool) -> Result<Packet, String> {
    if success {
        Ok(Packet { label: "hello 🦀".into(), data: vec![0, 128, 255], sequence: 9007199254740994, enabled: true })
    } else {
        Err("denied".into())
    }
}
pub fn relay(value: Result<Packet, String>) -> Result<Packet, String> { value }
pub fn numeric32(value: Result<f32, u32>) -> Result<f32, u32> { value }
pub fn numeric64(value: Result<f64, u64>) -> Result<f64, u64> { value }

pub struct Wide {
    pub a: String, pub b: String, pub c: String,
    pub d: String, pub e: String, pub f: String,
    pub g: String, pub h: String, pub i: String,
}
pub fn wide(value: Wide) -> Wide { value }
