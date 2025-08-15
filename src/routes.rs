use std::ops::Deref as _;
use std::sync::{Arc, RwLock};

use actix_web::web;
use actix_web::{get, post, HttpResponse, Responder};
use prost::Message;
use serde::{Deserialize, Serialize};

use crate::sharify;
use crate::sharify::room::{CredentialsInput, RoomError, RoomManager};

#[derive(Serialize, Deserialize)]
struct CreateRoom {
    client_id: String,
    username: String,
    name: String,
    credentials: CredentialsInput,
}

#[get("/")]
pub async fn root() -> impl Responder {
    HttpResponse::Ok()
}

#[post("/v1/create_room")]
pub async fn create_room(
    web::Json(CreateRoom {
        client_id,
        username,
        name,
        credentials,
    }): web::Json<CreateRoom>,
    sharify_state: web::Data<Arc<RwLock<RoomManager>>>,
) -> impl Responder {
    let mut state_guard = sharify_state.write().unwrap();
    debug!("{:?}", std::ptr::from_ref(state_guard.deref()));
    let room = match state_guard.create_room(client_id, username, name, credentials) {
        Ok(room) => room,
        Err(RoomError { error }) => {
            return HttpResponse::BadRequest().body(error);
        }
    };

    let proto_room = room.to_proto();

    drop(state_guard);

    let mut buf = Vec::new();
    if let Err(err) = proto_room.encode(&mut buf) {
        return HttpResponse::InternalServerError().body(format!(
            "Unexpected error while encoding newly created Room to protobuf: {err}"
        ));
    }

    HttpResponse::Created().body(buf)
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
