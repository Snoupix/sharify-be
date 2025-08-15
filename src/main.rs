#[macro_use]
extern crate log;

mod proto;
mod routes;
mod sharify;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::RwLock;
use std::sync::{Arc, Mutex, OnceLock};

use actix::SpawnHandle;
use actix_cors::Cors;
use actix_governor::{Governor, GovernorConfigBuilder};
use actix_web::middleware;
use actix_web::{middleware::Logger, web, App, HttpResponse, HttpServer};

use sharify::room::{RoomID, RoomManager};

use crate::sharify::websocket::{self, SharifyWsManager};

const SOCKET_ADDR: (u8, u8, u8, u8, u16) = (0, 0, 0, 0, 3100);

// static REFRESH_TOKEN_INTERVALS: OnceLock<Arc<Mutex<HashMap<RoomID, SpawnHandle>>>> =
//     OnceLock::new();
// static DATA_FETCHING_INTERVALS: OnceLock<Arc<Mutex<HashMap<RoomID, SpawnHandle>>>> =
//     OnceLock::new();

pub const DATA_FETCHING_INTERVAL: u64 = 5000;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenvy::dotenv().expect("failed to load .env file");

    env_logger::init_from_env(env_logger::Env::new().filter_or("LOG", "debug"));

    let sharify_ws_manager = Arc::new(RwLock::new(SharifyWsManager::default()));
    let sharify_state = Arc::new(RwLock::new(RoomManager::default()));

    // TODO: If behind a (reverse) proxy, change the key extractor because the peer IP will be the same
    // https://docs.rs/actix-governor/latest/actix_governor/struct.PeerIpKeyExtractor.html
    let governor_conf = GovernorConfigBuilder::default()
        .burst_size(10)
        .seconds_per_request(2)
        .finish()
        .expect("Failed to build governor (rate limiter)");

    HttpServer::new(move || {
        App::new()
            .wrap(
                Logger::new("%a/%{r}a %r status %s %Dms")
                    .exclude_regex("(/v1/[a-f0-9]{8}-.*|/v1/code.*)"),
            )
            .wrap(Cors::permissive()) // TODO prod: Change this
            .wrap(middleware::Compress::default())
            .wrap(Governor::new(&governor_conf))
            .app_data(web::Data::new(Arc::clone(&sharify_ws_manager)))
            .app_data(web::Data::new(Arc::clone(&sharify_state)))
            .default_service(web::to(HttpResponse::NotFound))
            .service(routes::root)
            .service(routes::create_room)
            .service(routes::code_verifier)
            .service(routes::code_challenge)
            .service(
                web::resource("/v1/{room_id}/{client_id}")
                    .route(web::get().to(websocket::SharifyWsInstance::init)),
            )
    })
    .bind((
        IpAddr::from(Ipv4Addr::new(
            SOCKET_ADDR.0,
            SOCKET_ADDR.1,
            SOCKET_ADDR.2,
            SOCKET_ADDR.3,
        )),
        SOCKET_ADDR.4,
    ))?
    .run()
    .await
}
