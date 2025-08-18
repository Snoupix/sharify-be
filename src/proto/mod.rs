pub mod cmd;
pub mod r#impl;
pub mod role;
pub mod room;
pub mod spotify;

use prost::Message as _;

pub fn create_error_response(error: impl Into<String>) -> Result<Vec<u8>, String> {
    let proto_cmd = cmd::CommandResponse {
        r#type: Some(cmd::command_response::Type::GenericError(error.into())),
    };

    let mut buf = Vec::new();
    if let Err(err) = proto_cmd.encode(&mut buf) {
        return Err(format!("Unexpected error while encoding newly created CommandResponse to protobuf command: {err}"));
    }

    Ok(buf)
}
