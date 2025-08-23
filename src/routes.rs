use std::sync::Arc;

use actix_web::web;
use actix_web::{get, post, HttpResponse, Responder};
use prost::Message as _;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::proto::cmd::{command_response, http_command, CommandResponse, HttpCommand};
use crate::proto::create_error_response;
use crate::sharify;
use crate::sharify::room::{CredentialsInput, RoomManager};
use crate::sharify::spotify::Timestamp;

#[get("/")]
pub async fn root() -> impl Responder {
    HttpResponse::Ok()
}

#[post("/v1")]
pub async fn proto_command(
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
            user_id,
            username,
            name,
            credentials: Some(credentials),
        }) => {
            let mut state_guard = sharify_state.write().await;
            let room = match state_guard.create_room(
                user_id,
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
                Err(error) => {
                    let proto_cmd: CommandResponse = error.into();

                    let mut buf = Vec::new();
                    proto_cmd.encode(&mut buf).unwrap();

                    return HttpResponse::BadRequest().body(buf);
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
        http_command::Type::GetRoom(http_command::GetRoom { room_id }) => {
            let state_guard = sharify_state.read().await;
            let Ok(uuid) = Uuid::from_slice(&room_id[..16]) else {
                return match create_error_response("Wrong UUID format") {
                    Err(err) => HttpResponse::InternalServerError().body(err),
                    Ok(buf) => HttpResponse::BadRequest().body(buf),
                };
            };
            let Some(room) = state_guard.get_room(&uuid) else {
                return HttpResponse::NotFound().finish();
            };

            let proto_command = CommandResponse {
                r#type: Some(command_response::Type::Room(room.clone().into())),
            };

            drop(state_guard);

            let mut buf = Vec::new();
            if let Err(err) = proto_command.encode(&mut buf) {
                return HttpResponse::InternalServerError().body(format!(
                    "Unexpected error while encoding newly created Room to protobuf command: {err}"
                ));
            }

            HttpResponse::Ok().body(buf)
        }
        http_command::Type::JoinRoom(http_command::JoinRoom {
            room_id,
            user_id,
            username,
        }) => {
            let mut state_guard = sharify_state.write().await;
            let Ok(uuid) = Uuid::from_slice(&room_id[..16]) else {
                return match create_error_response("Wrong UUID format") {
                    Err(err) => HttpResponse::InternalServerError().body(err),
                    Ok(buf) => HttpResponse::BadRequest().body(buf),
                };
            };
            let room = match state_guard.join_room(uuid, username, user_id) {
                Ok(room) => room,
                Err(err) => {
                    let mut buf = Vec::new();

                    CommandResponse::from(err).encode(&mut buf).unwrap();

                    return HttpResponse::Unauthorized().body(buf);
                }
            };

            drop(state_guard);

            let proto_command = CommandResponse {
                r#type: Some(command_response::Type::Room(room.into())),
            };

            let mut buf = Vec::new();
            if let Err(err) = proto_command.encode(&mut buf) {
                return HttpResponse::InternalServerError().body(format!(
                    "Unexpected error while encoding newly created Room to protobuf command: {err}"
                ));
            }

            HttpResponse::Ok().body(buf)
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
