#[macro_use]
extern crate log;

mod discord;
mod proto;
mod routes;
mod sharify;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use actix_cors::Cors;
use actix_governor::{Governor, GovernorConfigBuilder};
use actix_web::middleware;
use actix_web::{App, HttpResponse, HttpServer, middleware::Logger, web};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use tokio::sync::{Mutex, RwLock, mpsc};

use sharify::room::RoomID;
use sharify::room_manager::RoomManager;
use sharify::websocket::{self, SharifyWsManager};

const DEFAULT_SOCKET_ADDR: (Ipv4Addr, u16) = (Ipv4Addr::new(0, 0, 0, 0), 3100);

// static REFRESH_TOKEN_INTERVALS: OnceLock<Arc<Mutex<HashMap<RoomID, SpawnHandle>>>> =
//     OnceLock::new();
static DATA_FETCHING_INTERVALS: OnceLock<Arc<Mutex<HashMap<RoomID, mpsc::Sender<()>>>>> =
    OnceLock::new();

pub const DATA_FETCHING_INTERVAL: Duration = Duration::from_millis(5000);

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenvy::dotenv().expect("failed to load .env file");

    env_logger::init_from_env(env_logger::Env::new().filter_or("LOG", "debug"));

    let is_prod = dotenvy::var("IS_PROD")
        .map(|s| &s == "true")
        .unwrap_or(false);

    serve(is_prod).await
}

// Needed to be ran in tests
async fn serve(is_prod: bool) -> std::io::Result<()> {
    let sharify_ws_manager = Arc::new(RwLock::new(SharifyWsManager::default()));
    let sharify_state = Arc::new(RwLock::new(RoomManager::default()));

    // TODO: If behind a (reverse) proxy, change the key extractor because the peer IP will be the same
    // https://docs.rs/actix-governor/latest/actix_governor/struct.PeerIpKeyExtractor.html
    let governor_conf = GovernorConfigBuilder::default()
        .burst_size(10)
        .seconds_per_request(2)
        .finish()
        .expect("Failed to build governor (rate limiter)");

    let socket = (
        IpAddr::from(
            Ipv4Addr::from_str(&dotenvy::var("HOST").unwrap_or("".to_owned()))
                .unwrap_or(DEFAULT_SOCKET_ADDR.0),
        ),
        dotenvy::var("PORT")
            .map(|s| s.parse().expect("Failed to parse PORT env to valid u16"))
            .unwrap_or(DEFAULT_SOCKET_ADDR.1),
    );

    let server = HttpServer::new(move || {
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
            .service(routes::proto_command)
            .service(routes::code_verifier)
            .service(routes::code_challenge)
            .service(routes::send_discord_webhook)
            .service(
                web::resource("/v1/{room_id}/{user_id}")
                    .route(web::get().to(websocket::SharifyWsInstance::init)),
            )
    });

    match is_prod {
        true => {
            let key_path = dotenvy::var("TLS_PRIVATE_KEY").expect("TLS_PRIVATE_KEY env not found");
            let cert_path = dotenvy::var("TLS_CERT_KEY").expect("TLS_CERT_KEY env not found");

            let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls())?;

            builder.set_private_key_file(&key_path, SslFiletype::PEM)?;
            builder.set_certificate_chain_file(&cert_path)?;

            server.bind_openssl(socket, builder)?.run().await?;
        }
        false => {
            server.bind(socket)?.run().await?;
        }
    }

    Ok(())
}
