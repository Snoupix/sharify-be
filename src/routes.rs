use std::sync::{Arc, RwLock};

use actix_web::web;
use actix_web::{get, post, HttpResponse, Responder};
use prost::Message as _;

use crate::proto::cmd::{command_response, http_command, CommandResponse, HttpCommand};
use crate::sharify;
use crate::sharify::room::{CredentialsInput, RoomError, RoomManager};
use crate::sharify::spotify::Timestamp;

#[get("/")]
pub async fn root() -> impl Responder {
    HttpResponse::Ok()
}

#[post("/v1")]
pub async fn post_command(
    body: web::Payload,
    sharify_state: web::Data<Arc<RwLock<RoomManager>>>,
) -> impl Responder {
    let bad_request =
        HttpResponse::BadRequest().body("Failed to decode HTTP POST command with Protobuf");

    let Ok(Ok(command)) = body.to_bytes().await.map(HttpCommand::decode) else {
        return bad_request;
    };

    let Some(cmd_type) = command.r#type else {
        return bad_request;
    };

    match cmd_type {
        http_command::Type::CreateRoom(http_command::CreateRoom {
            client_id,
            username,
            name,
            credentials: Some(credentials),
        }) => {
            let mut state_guard = sharify_state.write().unwrap();
            let room = match state_guard.create_room(
                client_id,
                username,
                name,
                CredentialsInput {
                    access_token: credentials.access_token,
                    refresh_token: credentials.refresh_token,
                    expires_in: Timestamp::new(credentials.expires_in),
                    created_at: Timestamp::new(credentials.created_at),
                },
            ) {
                Ok(room) => room,
                Err(RoomError { error }) => {
                    return HttpResponse::BadRequest().body(error);
                }
            };

            let proto_command = CommandResponse {
                r#type: Some(command_response::Type::Room(room.into())),
            };

            drop(state_guard);

            let mut buf = Vec::new();
            if let Err(err) = proto_command.encode(&mut buf) {
                return HttpResponse::InternalServerError().body(format!(
                    "Unexpected error while encoding newly created Room to protobuf command: {err}"
                ));
            }

            HttpResponse::Created().body(buf)
        }
        _ => HttpResponse::ServiceUnavailable()
            .body("Unreachable error: POST command unhandled or missing command parts"),
    }
}

#[get("/v1/code_verifier")]
pub async fn code_verifier() -> impl Responder {
    HttpResponse::Ok().body(sharify::utils::generate_code_verifier())
}

#[get("/v1/code_challenge/{code_verifier}")]
pub async fn code_challenge(data: web::Path<String>) -> impl Responder {
    let _code_verifier = data.into_inner();
    HttpResponse::Ok().body(sharify::utils::generate_code_challenge(_code_verifier))
}
