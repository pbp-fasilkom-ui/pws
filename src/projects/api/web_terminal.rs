use std::{borrow::Cow, net::SocketAddr, time::Duration};

use axum::{
    extract::{
        ws::{CloseFrame, Message},
        ConnectInfo, Path, State, WebSocketUpgrade,
    },
    headers,
    http::StatusCode,
    response::IntoResponse,
    TypedHeader,
};
use bollard::{
    exec::{CreateExecOptions, StartExecResults},
    Docker,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::{auth::Auth, authz, startup::AppState};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WsRequest {
    pub message: String,
}

#[tracing::instrument(skip(auth, pool, ws))]
pub async fn ws(
    auth: Auth,
    State(AppState { pool, domain, .. }): State<AppState>,
    Path((owner, project)): Path<(String, String)>,
    ws: WebSocketUpgrade,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    origin: Option<TypedHeader<headers::Origin>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    // A WebSocket upgrade is not subject to the CORS layer, so the origin has
    // to be checked here. Without this, any page the user visits — including an
    // app deployed on a sibling subdomain, which is same-site and therefore
    // sends the session cookie — could open a shell as them.
    if !origin_is_allowed(origin.as_ref(), &domain) {
        tracing::warn!(?origin, "Rejected terminal upgrade from disallowed origin");
        return (StatusCode::FORBIDDEN, "Origin not allowed").into_response();
    }

    let Some(user) = auth.current_user else {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };

    // Being logged in is not enough: the session must be tied to *this*
    // project. Previously this handler took no user at all, so any account
    // could open a shell in any container.
    match authz::has_project_access(&pool, &owner, &project, user.id).await {
        Ok(true) => {}
        Ok(false) => {
            return (
                StatusCode::NOT_FOUND,
                "Project not found or you don't have access",
            )
                .into_response()
        }
        Err(err) => {
            tracing::error!(?err, "Can't start terminal: Failed to check project access");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to check access").into_response();
        }
    }

    // Derived from validated segments, and refuses names that would resolve to
    // one of the platform's own containers.
    let container_name = match authz::container_name(&owner, &project) {
        Ok(name) => name,
        Err(err) => {
            tracing::warn!(%owner, %project, %err, "Rejected terminal target");
            return (StatusCode::BAD_REQUEST, "Invalid project").into_response();
        }
    };

    let user_agent = if let Some(TypedHeader(user_agent)) = user_agent {
        user_agent.to_string()
    } else {
        String::from("Unknown browser")
    };

    // let who = SocketAddr::from(([127, 0, 0, 1], 0));
    let who = addr;

    tracing::info!(user_agent, "New websocket connection");

    ws.on_upgrade(move |mut socket| {
        async move {
            //send a ping (unsupported by some browsers) just to kick things off and get a response
            if socket.send(Message::Ping(vec![])).await.is_ok() {
                tracing::debug!(?who, "Pinged");
            } else {
                tracing::debug!(?who, "Could not send ping");
                return;
            }

            // receive single message from a client (we can either receive or send with socket).
            // this will likely be the Pong for our Ping or a hello message from client.
            // waiting for message from a client will block this task, but will not block other client's
            // connections.
            if let Some(msg) = socket.recv().await {
                if let Ok(msg) = msg {
                    if let Message::Close(c) = msg {
                        if let Some(cf) = c {
                            tracing::debug!(?who, code = cf.code, reason = ?cf.reason, "client disconected");
                        } else {
                            tracing::debug!(?who, "client disconected wihtout CloseFrame");
                        }
                    }
                } else {
                    println!("client {who} abruptly disconnected");
                    return;
                }
            }

            let docker = match Docker::connect_with_local_defaults() {
                Ok(docker) => docker,
                Err(err) => {
                    tracing::error!(?err, "Can't start terminal: Failed to connect to docker");
                    return;
                }
            };

            let exec = match docker
                .create_exec(
                    &container_name,
                    CreateExecOptions::<&str> {
                        attach_stdout: Some(true),
                        attach_stderr: Some(true),
                        attach_stdin: Some(true),
                        tty: Some(true),
                        cmd: Some(vec!["sh"]),
                        ..Default::default()
                    },
                )
                .await
            {
                Ok(exec) => exec,
                Err(err) => {
                    tracing::error!(?err, "Can't start terminal: Failed to create exec");
                    return;
                }
            };

            let (mut input, mut output)  =  match docker.start_exec(&exec.id, None).await {
                Ok(StartExecResults::Attached { output, input }) => (input , output),
                Ok(StartExecResults::Detached) => {
                    tracing::error!("Can't start terminal: Failed to start exec");
                    return;
                },
                Err(err) => {
                    tracing::error!(?err, "Can't start terminal: Failed to start exec");
                    return;
                }
            };

            // By splitting socket we can send and receive at the same time. In this example we will send
            let (mut sender, mut receiver) = socket.split();

            let mut send_task = tokio::spawn(async move {
                let mut i = 0;
                loop {

                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(10)) => {
                            if sender.send(Message::Ping(vec![])).await.is_err() {
                                break;
                            }
                        },
                        msg = output.next() => {
                            match msg {
                                Some(Ok(output)) => {
                                    let bytes = output.clone().into_bytes();
                                    let bytes = strip_ansi_escapes::strip(&bytes);
                                    let msg = String::from_utf8_lossy(&bytes);

                                    if sender
                                        .send(Message::Text(format!("{msg}")))
                                        .await
                                        .is_err()
                                    {
                                        break;
                                    }
                                    i += 1;
                                },
                                Some(Err(err)) => {
                                    tracing::error!(?err, "Can't receive message from terminal");
                                    break;
                                },
                                None => {
                                    tracing::error!("Can't receive message from terminal");
                                    break;
                                }
                            }
                        },

                    }
                }

                tracing::debug!(?who, "Sending close");
                if let Err(e) = sender
                    .send(Message::Close(Some(CloseFrame {
                        code: axum::extract::ws::close_code::NORMAL,
                        reason: Cow::from("Goodbye"),
                    })))
                    .await
                {
                    tracing::debug!(?e, "Could not send Close due to {e}");
                }
                i
            });

            // This second task will receive messages from client
            let mut recv_task = tokio::spawn({
                async move {
                    let mut cnt = 0;
                    while let Some(Ok(msg)) = receiver.next().await {
                        cnt += 1;
                        // print message and break if instructed to do so
                        match msg {
                            Message::Text(t) => {
                                match serde_json::from_str::<WsRequest>(&t) {
                                    Err(err) => {
                                        tracing::debug!(?err, "Can't parse message");
                                    },
                                    Ok(msg) => {
                                        let mut msg = msg.message;
                                        msg.push_str("\n");
                                        match input.write_all(msg.as_bytes()).await {
                                            Err(err) => {
                                                tracing::error!(?err, "Can't write to terminal");
                                                break;
                                            },
                                            Ok(_) => {
                                                // if let Err(err) = tx.send(WsMessage::Message(msg.message)).await {
                                                //     tracing::error!(?err, "Can't send message");
                                                // }
                                            }
                                        }
                                    }
                                };
                            }
                            Message::Close(c) => {
                                if let Some(cf) = c {
                                    tracing::debug!(?who, code = cf.code, reason = ?cf.reason, "client disconected");
                                } else {
                                    tracing::debug!(?who, "client disconected wihtout CloseFrame");
                                }
                                break;
                            }
                            _ => {}

                        }
                    }
                    cnt
            }});


            // If any one of the tasks exit, abort the other.
            tokio::select! {
                rv_a = (&mut send_task) => {
                    match rv_a {
                        Ok(a) => println!("{a} messages sent to {who}"),
                        Err(a) => println!("Error sending messages {a:?}")
                    }
                    recv_task.abort();
                },
                rv_b = (&mut recv_task) => {
                    match rv_b {
                        Ok(b) => println!("Received {b} messages"),
                        Err(b) => println!("Error receiving messages {b:?}")
                    }
                    send_task.abort();
                },
            }

            // returning from the handler closes the websocket connection
            tracing::info!(?who, "Websocket context destroyed");
        }
    })
    .into_response()
}

/// Mirrors the CORS allow-list in `startup::run`, which WebSocket upgrades
/// bypass. A missing `Origin` header is allowed so that non-browser clients
/// (which are not subject to CSRF) keep working; browsers always send one.
fn origin_is_allowed(origin: Option<&TypedHeader<headers::Origin>>, domain: &str) -> bool {
    let Some(TypedHeader(origin)) = origin else {
        return true;
    };

    let origin = origin.to_string();

    [
        "http://localhost:8080".to_string(),
        "http://localhost:5173".to_string(),
        format!("https://{domain}"),
        format!("http://{domain}"),
    ]
    .iter()
    .any(|allowed| allowed == &origin)
}
