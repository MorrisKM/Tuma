//! gRPC end to end: a tonic server built from `fixtures/grpc/shop.proto` (no generated code: it uses the same dynamic
//! codec), with reflection, called by Tuma's client for each kind of call.

use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;

use futures_util::StreamExt;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use prost::Message as _;
use prost_reflect::{DescriptorPool, DynamicMessage, Value as PValue};
use tokio::net::TcpListener;
use tonic::{Code, Status};
use tuma_core::grpc::{self, DynamicCodec, GrpcEvent, GrpcSettings, Target};
use tuma_core::http::{Network, Request};

type Response = hyper::Response<tonic::body::Body>;
type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;

fn fixtures() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/grpc")
}

fn pool() -> (DescriptorPool, Vec<u8>) {
    let set = protox::compile([fixtures().join("shop.proto")], [fixtures()]).unwrap();
    let bytes = set.encode_to_vec();
    (
        DescriptorPool::from_file_descriptor_set(set).unwrap(),
        bytes,
    )
}

fn item(pool: &DescriptorPool, id: i64, name: &str, price: f64) -> DynamicMessage {
    let mut m = DynamicMessage::new(pool.get_message_by_name("shop.v1.Item").unwrap());
    m.set_field_by_name("id", PValue::I64(id));
    m.set_field_by_name("name", PValue::String(name.into()));
    m.set_field_by_name("price", PValue::F64(price));
    m
}

type ItemStream =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<DynamicMessage, Status>> + Send>>;

struct GetItem(Arc<DescriptorPool>);
impl tonic::server::UnaryService<DynamicMessage> for GetItem {
    type Response = DynamicMessage;
    type Future = BoxFuture<Result<tonic::Response<DynamicMessage>, Status>>;
    fn call(&mut self, request: tonic::Request<DynamicMessage>) -> Self::Future {
        let pool = self.0.clone();
        Box::pin(async move {
            if request
                .metadata()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                != Some("Bearer t0k")
            {
                return Err(Status::unauthenticated("no token"));
            }
            let id = request
                .get_ref()
                .get_field_by_name("id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            if id != 7 {
                return Err(Status::not_found(format!("no item {id}")));
            }
            let mut response = tonic::Response::new(item(&pool, 7, "Dune", 9.5));
            response
                .metadata_mut()
                .insert("x-shop", "open".parse().unwrap());
            Ok(response)
        })
    }
}

struct ListItems(Arc<DescriptorPool>);
impl tonic::server::ServerStreamingService<DynamicMessage> for ListItems {
    type Response = DynamicMessage;
    type ResponseStream = ItemStream;
    type Future = BoxFuture<Result<tonic::Response<ItemStream>, Status>>;
    fn call(&mut self, request: tonic::Request<DynamicMessage>) -> Self::Future {
        let pool = self.0.clone();
        Box::pin(async move {
            let limit = request
                .get_ref()
                .get_field_by_name("limit")
                .and_then(|v| v.as_i32())
                .unwrap_or(0);
            let items: Vec<Result<DynamicMessage, Status>> = (1..=limit as i64)
                .map(|i| Ok(item(&pool, i, &format!("item {i}"), i as f64)))
                .collect();
            Ok(tonic::Response::new(
                Box::pin(futures_util::stream::iter(items)) as ItemStream,
            ))
        })
    }
}

struct AddItems(Arc<DescriptorPool>);
impl tonic::server::ClientStreamingService<DynamicMessage> for AddItems {
    type Response = DynamicMessage;
    type Future = BoxFuture<Result<tonic::Response<DynamicMessage>, Status>>;
    fn call(&mut self, request: tonic::Request<tonic::Streaming<DynamicMessage>>) -> Self::Future {
        let pool = self.0.clone();
        Box::pin(async move {
            let mut stream = request.into_inner();
            let (mut count, mut total) = (0, 0.0);
            while let Some(item) = stream.message().await? {
                count += 1;
                total += item
                    .get_field_by_name("price")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
            }
            let mut summary =
                DynamicMessage::new(pool.get_message_by_name("shop.v1.Summary").unwrap());
            summary.set_field_by_name("count", PValue::I32(count));
            summary.set_field_by_name("total", PValue::F64(total));
            Ok(tonic::Response::new(summary))
        })
    }
}

struct Chat(Arc<DescriptorPool>);
impl tonic::server::StreamingService<DynamicMessage> for Chat {
    type Response = DynamicMessage;
    type ResponseStream = ItemStream;
    type Future = BoxFuture<Result<tonic::Response<ItemStream>, Status>>;
    fn call(&mut self, request: tonic::Request<tonic::Streaming<DynamicMessage>>) -> Self::Future {
        let pool = self.0.clone();
        Box::pin(async move {
            let stream = request.into_inner().map(move |note| {
                let note = note?;
                let text = note
                    .get_field_by_name("text")
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let mut reply =
                    DynamicMessage::new(pool.get_message_by_name("shop.v1.Note").unwrap());
                reply.set_field_by_name("text", PValue::String(format!("you said {text}")));
                Ok(reply)
            });
            Ok(tonic::Response::new(Box::pin(stream) as ItemStream))
        })
    }
}

async fn route(
    pool: Arc<DescriptorPool>,
    reflection: tonic_reflection::server::v1::ServerReflectionServer<
        impl tonic_reflection::server::v1::ServerReflection,
    >,
    request: hyper::Request<hyper::body::Incoming>,
) -> Result<Response, Infallible> {
    let request = request.map(tonic::body::Body::new);
    let method = |name: &str| {
        pool.get_service_by_name("shop.v1.Shop")
            .unwrap()
            .methods()
            .find(|m| m.name() == name)
            .unwrap()
    };
    let codec = |name: &str| DynamicCodec::new(method(name).input());
    let path = request.uri().path().to_string();
    let response = match path.as_str() {
        "/shop.v1.Shop/GetItem" => {
            tonic::server::Grpc::new(codec("GetItem"))
                .unary(GetItem(pool.clone()), request)
                .await
        }
        "/shop.v1.Shop/ListItems" => {
            tonic::server::Grpc::new(codec("ListItems"))
                .server_streaming(ListItems(pool.clone()), request)
                .await
        }
        "/shop.v1.Shop/AddItems" => {
            tonic::server::Grpc::new(codec("AddItems"))
                .client_streaming(AddItems(pool.clone()), request)
                .await
        }
        "/shop.v1.Shop/Chat" => {
            tonic::server::Grpc::new(codec("Chat"))
                .streaming(Chat(pool.clone()), request)
                .await
        }
        _ if path.starts_with("/grpc.reflection.v1.") => {
            let mut reflection = reflection;
            return Ok(tower::Service::call(&mut reflection, request)
                .await
                .unwrap());
        }
        _ => Status::unimplemented("no such method").into_http(),
    };
    Ok(response)
}

async fn server() -> String {
    let (pool, bytes) = pool();
    let pool = Arc::new(pool);
    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(&bytes)
        .build_v1()
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let pool = pool.clone();
            let reflection = reflection.clone();
            tokio::spawn(async move {
                let _ = auto::Builder::new(TokioExecutor::new())
                    .serve_connection(
                        TokioIo::new(stream),
                        service_fn(move |r| route(pool.clone(), reflection.clone(), r)),
                    )
                    .await;
            });
        }
    });
    format!("grpc://{addr}")
}

async fn target(url: &str, token: Option<&str>) -> Target {
    let request = Request {
        url: url.into(),
        auth: match token {
            Some(t) => tuma_core::http::Auth::Bearer { token: t.into() },
            None => tuma_core::http::Auth::None,
        },
        ..Default::default()
    };
    Target::from_request(&request, Arc::new(Network::default()))
        .await
        .unwrap()
}

#[tokio::test]
async fn reflection_and_every_kind_of_call() {
    let url = server().await;
    let target = target(&url, Some("t0k")).await;
    // Reflection, with the well-known Timestamp type sent along as a dependency.
    let pool = grpc::describe(&target, &GrpcSettings::default(), None)
        .await
        .unwrap();
    let services = grpc::services(&pool);
    assert_eq!(services.len(), 1, "{services:?}");
    let methods: Vec<(&str, bool, bool)> = services[0]
        .methods
        .iter()
        .map(|m| (m.name.as_str(), m.client_streaming, m.server_streaming))
        .collect();
    assert_eq!(
        methods,
        [
            ("GetItem", false, false),
            ("ListItems", false, true),
            ("AddItems", true, false),
            ("Chat", true, true)
        ]
    );
    let example: serde_json::Value = serde_json::from_str(&services[0].methods[2].example).unwrap();
    assert_eq!(example["id"], "0");
    assert_eq!(example["kind"], "KIND_UNSPECIFIED");
    assert_eq!(example["added"], "1970-01-01T00:00:00Z");

    // Unary, with metadata both ways.
    let done = grpc::call(
        &target,
        &pool,
        "shop.v1.Shop/GetItem",
        &[r#"{"id": "7"}"#.into()],
    )
    .await
    .unwrap();
    assert_eq!(done.code, Code::Ok, "{}", done.message);
    let item: serde_json::Value = serde_json::from_str(&done.messages[0]).unwrap();
    assert_eq!(item["name"], "Dune");
    assert_eq!(item["price"], 9.5);
    assert!(
        done.headers
            .iter()
            .any(|h| h.name == "x-shop" && h.value == "open")
    );
    let response = grpc::as_response(&url, &done);
    assert_eq!((response.status, response.reason.as_str()), (200, "OK"));

    // Errors come back as statuses.
    let missing = grpc::call(
        &target,
        &pool,
        "shop.v1.Shop/GetItem",
        &[r#"{"id": 1}"#.into()],
    )
    .await
    .unwrap();
    assert_eq!(
        (missing.code, missing.message.as_str()),
        (Code::NotFound, "no item 1")
    );
    assert_eq!(grpc::as_response(&url, &missing).status, 404);
    let anonymous = target_without_token(&url).await;
    let refused = grpc::call(
        &anonymous,
        &pool,
        "shop.v1.Shop/GetItem",
        &[r#"{"id": 7}"#.into()],
    )
    .await
    .unwrap();
    assert_eq!(refused.code, Code::Unauthenticated);

    // Server streaming, client streaming, bidirectional.
    let listed = grpc::call(
        &target,
        &pool,
        "shop.v1.Shop/ListItems",
        &[r#"{"limit": 3}"#.into()],
    )
    .await
    .unwrap();
    assert_eq!(listed.messages.len(), 3);
    let body: serde_json::Value =
        serde_json::from_slice(&grpc::as_response(&url, &listed).body).unwrap();
    assert_eq!(body[2]["name"], "item 3");
    let summary = grpc::call(
        &target,
        &pool,
        "shop.v1.Shop/AddItems",
        &[
            r#"{"price": 1.5}"#.into(),
            r#"{"price": 2}"#.into(),
            r#"{"name": "x", "price": 0.5}"#.into(),
        ],
    )
    .await
    .unwrap();
    let summary: serde_json::Value = serde_json::from_str(&summary.messages[0]).unwrap();
    assert_eq!(
        (summary["count"].as_i64(), summary["total"].as_f64()),
        (Some(3), Some(4.0))
    );
    let chat = grpc::call(
        &target,
        &pool,
        "shop.v1.Shop/Chat",
        &[r#"{"text": "a"}"#.into(), r#"{"text": "b"}"#.into()],
    )
    .await
    .unwrap();
    assert_eq!(chat.messages.len(), 2);
    assert!(chat.messages[1].contains("you said b"));

    // Bad input is caught before sending.
    let error = grpc::call(
        &target,
        &pool,
        "shop.v1.Shop/GetItem",
        &[r#"{"idd": 7}"#.into()],
    )
    .await
    .unwrap_err();
    assert!(
        error.contains("doesn't fit shop.v1.GetItemRequest"),
        "{error}"
    );
    let error = grpc::call(&target, &pool, "shop.v1.Shop/Nope", &[])
        .await
        .unwrap_err();
    assert!(error.contains("has no method Nope"), "{error}");
}

async fn target_without_token(url: &str) -> Target {
    target(url, None).await
}

#[tokio::test]
async fn proto_files_and_live_streams() {
    let url = server().await;
    let target = target(&url, Some("t0k")).await;
    let settings = GrpcSettings {
        method: "shop.v1.Shop/Chat".into(),
        protos: vec!["shop.proto".into()],
        imports: vec![],
    };
    let pool = grpc::describe(&target, &settings, Some(&fixtures()))
        .await
        .unwrap();
    assert_eq!(grpc::services(&pool)[0].methods.len(), 4);

    // A live bidirectional call: messages go out one by one and answers come back before the stream ends.
    let events: Arc<std::sync::Mutex<Vec<GrpcEvent>>> = Arc::default();
    let record = events.clone();
    let mut call = grpc::start(
        &target,
        &pool,
        &settings.method,
        Arc::new(move |e| record.lock().unwrap().push(e)),
    )
    .await
    .unwrap();
    call.send(r#"{"text": "one"}"#).unwrap();
    for _ in 0..100 {
        if events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, GrpcEvent::Message { incoming: true, .. }))
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(events.lock().unwrap().iter().any(|e| matches!(e, GrpcEvent::Message { incoming: true, json, .. } if json.contains("you said one"))));
    call.send(r#"{"text": "two"}"#).unwrap();
    call.end();
    for _ in 0..100 {
        if events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, GrpcEvent::Done { .. }))
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let events = events.lock().unwrap().clone();
    assert!(
        matches!(events.last(), Some(GrpcEvent::Done { code: 0, .. })),
        "{events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, GrpcEvent::Message { incoming: true, .. }))
            .count(),
        2
    );

    // A .proto that doesn't compile says why.
    let broken = GrpcSettings {
        protos: vec!["missing.proto".into()],
        ..settings
    };
    let error = grpc::describe(&target, &broken, Some(&fixtures()))
        .await
        .unwrap_err();
    assert!(error.contains("don't compile"), "{error}");
}
