use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use actix_rt::time::timeout;
use futures_util::{SinkExt as _, TryStreamExt as _};
use prost::Message as _;
use reqwest::{Client, ClientBuilder, StatusCode};
use reqwest_websocket::{CloseCode, Message, RequestBuilderExt};
use tokio::sync::mpsc;

use crate::proto::cmd::{
    command, command_response, http_command, Command, CommandResponse, HttpCommand,
};
use crate::sharify::room::Room;
use crate::sharify::utils;

const BASE_URL: &str = "http://127.0.0.1:3100/v1";

static NEXT_ROOM_ID: AtomicU8 = AtomicU8::new(1);

async fn run_server_with_timeout(seconds: u64, mut cancel_rx: mpsc::Receiver<()>) {
    actix_rt::spawn(async move {
        tokio::select! {
            timeout = timeout(Duration::from_secs(seconds), crate::serve()) => {
                if timeout.is_err() {
                    panic!("Timeout hit during test");
                }
            },
            _ = cancel_rx.recv() => {},
        }
    });

    // Await for server start
    for _ in 0..3 {
        if Client::default()
            .get(BASE_URL)
            .timeout(Duration::from_millis(1000))
            .send()
            .await
            .is_ok()
        {
            break;
        }
    }
}

async fn create_room_impl(sv_timeout: u64) -> (mpsc::Sender<()>, Client, Room) {
    let (cancel_tx, cancel_rx) = mpsc::channel::<()>(1);
    run_server_with_timeout(sv_timeout, cancel_rx).await;

    let client = ClientBuilder::default()
        .timeout(Duration::from_secs(60 * 2))
        .build()
        .unwrap();

    let command = HttpCommand {
        r#type: Some(http_command::Type::CreateRoom(http_command::CreateRoom {
            client_id: utils::encode_user_email(
                format!(
                    "test{}@email.com",
                    NEXT_ROOM_ID.fetch_add(1, Ordering::SeqCst)
                ),
                10,
            ),
            username: "test".into(),
            name: format!("Room {}", NEXT_ROOM_ID.fetch_add(1, Ordering::SeqCst)),
            credentials: Some(http_command::Credentials {
                access_token: "".into(),
                refresh_token: "".into(),
                expires_in: "".into(),
                created_at: "".into(),
            }),
        })),
    };

    let mut buf = Vec::new();
    assert!(
        command.encode(&mut buf).is_ok(),
        "Failed to encode HTTPCommand to buffer"
    );

    let req = client
        .post(BASE_URL)
        .body(buf)
        .send()
        .await
        .expect("Failed to send CreateRoom POST request");

    assert_eq!(req.status(), StatusCode::CREATED);

    let res = CommandResponse::decode(req.bytes().await.expect("Failed to get response bytes"))
        .expect("Failed to decode respones into Protobuf CommandResponse");

    assert!(res
        .r#type
        .as_ref()
        .is_some_and(|t| matches!(t, command_response::Type::Room(_))));

    let command_response::Type::Room(room) = res.r#type.unwrap() else {
        unreachable!();
    };

    (cancel_tx, client, room.into())
}

#[actix_rt::test]
async fn create_room() {
    create_room_impl(60 * 2).await;
}

#[actix_rt::test]
async fn create_room_and_get_room_via_ws() {
    let (cancel_tx, client, room) = create_room_impl(60 * 4).await;

    let req = client
        .get(format!("{BASE_URL}/{}/{}", room.id, room.clients[0].id))
        .upgrade()
        .send()
        .await
        .expect("Failed to send HTTP GET request to create WS conn");

    assert_eq!(req.status(), StatusCode::SWITCHING_PROTOCOLS);

    let mut ws = req
        .into_websocket()
        .await
        .expect("Failed to upgrade HTTP request to WS");

    let command = Command {
        r#type: Some(command::Type::GetRoom(false)),
    };

    let mut buf = Vec::new();
    assert!(
        command.encode(&mut buf).is_ok(),
        "Failed to encode WS Command to buffer"
    );

    assert!(
        ws.send(buf.into()).await.is_ok(),
        "Failed to send Command message to WS"
    );

    while let Some(res) = ws
        .try_next()
        .await
        .expect("Failed to get WS response to GetRoom Command")
    {
        if matches!(res, Message::Ping(_)) {
            continue;
        }

        assert!(
            matches!(res, Message::Binary(_)),
            "Received WS message is not expected"
        );

        let Message::Binary(bytes) = res else {
            unreachable!();
        };

        let cmd = CommandResponse::decode(bytes)
            .expect("Failed to decode received bytes into CommandResponse");

        assert!(cmd
            .r#type
            .as_ref()
            .is_some_and(|t| matches!(t, command_response::Type::Room(_))));

        let Some(command_response::Type::Room(proto_room)) = cmd.r#type else {
            unreachable!();
        };

        let received_room: Room = proto_room.into();

        assert_eq!(room.id, received_room.id);

        let _ = ws.close(CloseCode::Normal, None).await;

        let _ = cancel_tx.send(()).await;

        return;
    }

    let _ = cancel_tx.send(()).await;
    unreachable!("If this is triggered, this means that WS conn has been closed");
}
