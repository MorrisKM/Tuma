//! gRPC calls without generated code: methods are described by server reflection or by `.proto` files compiled at
//! runtime, and messages are written and shown as JSON. Calls go over Tuma's own connections (proxy tunnel, client
//! certificates, extra CAs), and every kind of call (unary, server, client and bidirectional streaming) is one stream
//! of messages each way.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use bytes::Buf;
use prost::Message as _;
use prost_reflect::{
    DescriptorPool, DynamicMessage, FieldDescriptor, Kind, MessageDescriptor, MethodDescriptor,
    SerializeOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::transport::{Channel, Endpoint};
use tonic::{Code, Status};
use url::Url;

use crate::http::{self, Header, Network};

/// Where a request's method descriptions come from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct GrpcSettings {
    /// The method to call, as `package.Service/Method`.
    pub method: String,
    /// `.proto` files, relative to the workspace. Empty: ask the server (reflection).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub protos: Vec<String>,
    /// Folders imports are found in, relative to the workspace. The files' own folders are searched too.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<String>,
}

/// A service and its methods, for pickers.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInfo {
    pub name: String,
    pub methods: Vec<MethodInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MethodInfo {
    pub name: String,
    /// `package.Service/Method`.
    pub path: String,
    pub client_streaming: bool,
    pub server_streaming: bool,
    pub input: String,
    pub output: String,
    /// A JSON message with every field, as a starting point.
    pub example: String,
}

/// What happens during a call.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GrpcEvent {
    /// The server's response headers (metadata).
    Headers { metadata: Vec<Header> },
    /// A message, sent or received, as JSON.
    #[serde(rename_all = "camelCase")]
    Message {
        incoming: bool,
        json: String,
        at: u64,
    },
    /// The call ended, with its status and trailers.
    #[serde(rename_all = "camelCase")]
    Done {
        code: i32,
        /// Such as `OK` or `NOT_FOUND`.
        status: String,
        message: String,
        metadata: Vec<Header>,
        elapsed_ms: u64,
    },
}

pub type EventSink = Arc<dyn Fn(GrpcEvent) + Send + Sync>;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// The gRPC status name, as in the spec.
pub fn code_name(code: Code) -> &'static str {
    match code {
        Code::Ok => "OK",
        Code::Cancelled => "CANCELLED",
        Code::Unknown => "UNKNOWN",
        Code::InvalidArgument => "INVALID_ARGUMENT",
        Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
        Code::NotFound => "NOT_FOUND",
        Code::AlreadyExists => "ALREADY_EXISTS",
        Code::PermissionDenied => "PERMISSION_DENIED",
        Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
        Code::FailedPrecondition => "FAILED_PRECONDITION",
        Code::Aborted => "ABORTED",
        Code::OutOfRange => "OUT_OF_RANGE",
        Code::Unimplemented => "UNIMPLEMENTED",
        Code::Internal => "INTERNAL",
        Code::Unavailable => "UNAVAILABLE",
        Code::DataLoss => "DATA_LOSS",
        Code::Unauthenticated => "UNAUTHENTICATED",
    }
}

/// The HTTP status closest to a gRPC status, so checks and tests on gRPC requests read like HTTP ones.
pub fn http_status(code: Code) -> u16 {
    match code {
        Code::Ok => 200,
        Code::InvalidArgument | Code::FailedPrecondition | Code::OutOfRange => 400,
        Code::Unauthenticated => 401,
        Code::PermissionDenied => 403,
        Code::NotFound => 404,
        Code::AlreadyExists | Code::Aborted => 409,
        Code::ResourceExhausted => 429,
        Code::Cancelled => 499,
        Code::Unimplemented => 501,
        Code::Unavailable => 503,
        Code::DeadlineExceeded => 504,
        _ => 500,
    }
}

// The codec: messages are DynamicMessages of a known type.

#[derive(Clone)]
pub struct DynamicCodec {
    response: MessageDescriptor,
}

impl DynamicCodec {
    pub fn new(response: MessageDescriptor) -> Self {
        Self { response }
    }
}

pub struct DynamicEncoder;
pub struct DynamicDecoder(MessageDescriptor);

impl Codec for DynamicCodec {
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;
    type Encoder = DynamicEncoder;
    type Decoder = DynamicDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        DynamicEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        DynamicDecoder(self.response.clone())
    }
}

impl Encoder for DynamicEncoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        item.encode(dst)
            .map_err(|e| Status::internal(e.to_string()))
    }
}

impl Decoder for DynamicDecoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        let bytes = src.copy_to_bytes(src.remaining());
        DynamicMessage::decode(self.0.clone(), bytes)
            .map(Some)
            .map_err(|e| Status::internal(format!("could not decode the response: {e}")))
    }
}

// Connecting

/// Where to call and how: `grpc://` or `grpcs://` (or `http://`, `https://`) and the metadata to send.
#[derive(Clone)]
pub struct Target {
    pub url: Url,
    pub verify_tls: bool,
    pub metadata: Vec<(String, String)>,
    pub network: Arc<Network>,
}

impl Target {
    /// From a resolved request: its URL, TLS setting, and headers and auth as metadata.
    pub async fn from_request(
        request: &http::Request,
        network: Arc<Network>,
    ) -> Result<Self, String> {
        let raw = request.url.trim();
        let (secure, rest) = match raw.split_once("://") {
            Some(("grpcs" | "https", rest)) => (true, rest),
            Some(("grpc" | "http", rest)) => (false, rest),
            Some((other, _)) => {
                return Err(format!(
                    "{other}:// is not a gRPC address; use grpc:// or grpcs://"
                ));
            }
            None => (false, raw),
        };
        let as_http = http::Request {
            method: "POST".into(),
            url: format!("{}://{rest}", if secure { "https" } else { "http" }),
            body: http::Body::None,
            ..request.clone()
        };
        let prepared = http::prepare(&as_http).await.map_err(|e| e.to_string())?;
        // HTTP/2 framing headers and the request's own defaults are not metadata.
        const SKIP: [&str; 8] = [
            "host",
            "user-agent",
            "accept",
            "accept-encoding",
            "content-type",
            "content-length",
            "te",
            "connection",
        ];
        let metadata = prepared
            .headers
            .iter()
            .filter(|(n, _)| !SKIP.contains(&n.as_str()))
            .map(|(n, v)| {
                (
                    n.to_string(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect();
        Ok(Self {
            url: prepared.url,
            verify_tls: request.settings.verify_tls,
            metadata,
            network,
        })
    }

    async fn channel(&self) -> Result<Channel, String> {
        let url = self.url.clone();
        let origin = format!(
            "{}://{}",
            url.scheme(),
            &url[url::Position::BeforeHost..url::Position::AfterPort]
        );
        let endpoint =
            Endpoint::from_shared(origin.clone()).map_err(|e| format!("{origin}: {e}"))?;
        let network = self.network.clone();
        let verify = self.verify_tls;
        let connector = tower::service_fn(move |_: ::http::Uri| {
            let url = url.clone();
            let network = network.clone();
            async move { connect(&url, &network, verify).await }
        });
        endpoint
            .connect_with_connector(connector)
            .await
            .map_err(|e| format!("could not connect to {origin}: {}", error_chain(&e)))
    }

    fn request<T>(&self, message: T) -> Result<tonic::Request<T>, String> {
        let mut request = tonic::Request::new(message);
        for (name, value) in &self.metadata {
            let key = tonic::metadata::MetadataKey::from_bytes(name.as_bytes())
                .map_err(|_| format!("\"{name}\" can't be gRPC metadata"))?;
            let value = value
                .parse()
                .map_err(|_| format!("the value of \"{name}\" can't be gRPC metadata"))?;
            request.metadata_mut().append(key, value);
        }
        Ok(request)
    }
}

trait Io: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Io for T {}

/// A connection to a gRPC server: plain HTTP/2 for grpc://, TLS with HTTP/2 for grpcs://, through the proxy if any.
async fn connect(
    url: &Url,
    network: &Network,
    verify: bool,
) -> Result<hyper_util::rt::TokioIo<Box<dyn Io>>, std::io::Error> {
    let io_error = |e: http::Error| std::io::Error::other(e.to_string());
    let secure = url.scheme() == "https";
    let host = url.host_str().unwrap_or_default().to_string();
    let port = url.port_or_known_default().unwrap_or(80);
    let proxy = network.proxy.as_ref().and_then(|p| p.for_url(url));
    let mut timing = http::Timing::default();
    let mut tcp = http::connect_to(proxy.unwrap_or(url), &mut timing)
        .await
        .map_err(io_error)?;
    if let Some(proxy) = proxy {
        http::tunnel(
            &mut tcp,
            &host,
            port,
            http::proxy_authorization(proxy).as_deref(),
        )
        .await
        .map_err(io_error)?;
    }
    if !secure {
        return Ok(hyper_util::rt::TokioIo::new(Box::new(tcp)));
    }
    let config = http::tls::config(http::tls::Options {
        verify,
        client_certificate: network.client_certificate(&host, Some(port)),
        ca_certificates: &network.ca_certificates,
        http1_only: false,
    })
    .map_err(io_error)?;
    let name =
        rustls::pki_types::ServerName::try_from(host.clone()).map_err(std::io::Error::other)?;
    let tls = tokio_rustls::TlsConnector::from(config)
        .connect(name, tcp)
        .await?;
    Ok(hyper_util::rt::TokioIo::new(Box::new(tls)))
}

/// An error and its causes, which tonic nests deeply.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut out = error.to_string();
    let mut source = error.source();
    while let Some(e) = source {
        let text = e.to_string();
        if !out.contains(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        source = e.source();
    }
    out
}

// Descriptions

/// The method descriptions: from `.proto` files (paths relative to `root`), or from the server by reflection.
pub async fn describe(
    target: &Target,
    settings: &GrpcSettings,
    root: Option<&Path>,
) -> Result<DescriptorPool, String> {
    if settings.protos.is_empty() {
        return reflect(target).await;
    }
    let base = root.map(Path::to_path_buf).unwrap_or_default();
    let files: Vec<PathBuf> = settings.protos.iter().map(|p| base.join(p)).collect();
    let mut includes: Vec<PathBuf> = settings.imports.iter().map(|p| base.join(p)).collect();
    for file in &files {
        if let Some(dir) = file.parent()
            && !includes.contains(&dir.to_path_buf())
        {
            includes.push(dir.to_path_buf());
        }
    }
    let set = protox::compile(&files, &includes)
        .map_err(|e| format!("the .proto files don't compile: {e}"))?;
    DescriptorPool::from_file_descriptor_set(set).map_err(|e| e.to_string())
}

/// Asks the server for its services and their files (gRPC server reflection v1, then v1alpha).
async fn reflect(target: &Target) -> Result<DescriptorPool, String> {
    use tonic_reflection::pb::v1;
    let channel = target.channel().await?;
    let mut client = v1::server_reflection_client::ServerReflectionClient::new(channel.clone());
    let ask =
        |request: v1::server_reflection_request::MessageRequest| v1::ServerReflectionRequest {
            host: String::new(),
            message_request: Some(request),
        };
    let one = |client: &mut v1::server_reflection_client::ServerReflectionClient<Channel>,
               request: v1::ServerReflectionRequest| {
        let request = target.request(tokio_stream::once(request));
        let mut client = client.clone();
        async move {
            let response = client
                .server_reflection_info(request?)
                .await
                .map_err(|s| status_text(&s))?;
            let mut stream = response.into_inner();
            stream
                .next()
                .await
                .ok_or("the reflection service closed the stream")?
                .map_err(|s| status_text(&s))
        }
    };
    let services = match one(
        &mut client,
        ask(v1::server_reflection_request::MessageRequest::ListServices(
            String::new(),
        )),
    )
    .await
    {
        Ok(response) => response,
        Err(e) if e.contains("UNIMPLEMENTED") => return reflect_v1alpha(target).await,
        Err(e) => {
            return Err(format!(
                "server reflection failed: {e}. Add the service's .proto files instead"
            ));
        }
    };
    let names: Vec<String> = match services.message_response {
        Some(v1::server_reflection_response::MessageResponse::ListServicesResponse(list)) => list
            .service
            .into_iter()
            .map(|s| s.name)
            .filter(|n| !n.starts_with("grpc.reflection."))
            .collect(),
        _ => return Err("the server's reflection answer has no services".into()),
    };
    let mut files: Vec<prost_types::FileDescriptorProto> = Vec::new();
    let mut wanted: Vec<v1::server_reflection_request::MessageRequest> = names
        .iter()
        .map(|n| v1::server_reflection_request::MessageRequest::FileContainingSymbol(n.clone()))
        .collect();
    // Files arrive with their dependencies, or the dependencies are asked for by name.
    while let Some(request) = wanted.pop() {
        let response = one(&mut client, ask(request)).await?;
        let Some(v1::server_reflection_response::MessageResponse::FileDescriptorResponse(found)) =
            response.message_response
        else {
            continue;
        };
        for bytes in found.file_descriptor_proto {
            let file = prost_types::FileDescriptorProto::decode(bytes.as_slice())
                .map_err(|e| e.to_string())?;
            if files.iter().any(|f| f.name == file.name) {
                continue;
            }
            for dependency in &file.dependency {
                if !files
                    .iter()
                    .any(|f| f.name.as_deref() == Some(dependency.as_str()))
                {
                    wanted.push(
                        v1::server_reflection_request::MessageRequest::FileByFilename(
                            dependency.clone(),
                        ),
                    );
                }
            }
            files.push(file);
        }
    }
    pool_from(files)
}

async fn reflect_v1alpha(target: &Target) -> Result<DescriptorPool, String> {
    use tonic_reflection::pb::v1alpha as v;
    let channel = target.channel().await?;
    let mut client = v::server_reflection_client::ServerReflectionClient::new(channel);
    let mut ask = async |request: v::server_reflection_request::MessageRequest| -> Result<v::ServerReflectionResponse, String> {
        let request = target.request(tokio_stream::once(v::ServerReflectionRequest {
            host: String::new(),
            message_request: Some(request),
        }))?;
        let mut stream = client.server_reflection_info(request).await.map_err(|s| status_text(&s))?.into_inner();
        stream.next().await.ok_or("the reflection service closed the stream")?.map_err(|s| status_text(&s))
    };
    let names: Vec<String> = match ask(v::server_reflection_request::MessageRequest::ListServices(
        String::new(),
    ))
    .await
    .map_err(|e| format!("server reflection failed: {e}. Add the service's .proto files instead"))?
    .message_response
    {
        Some(v::server_reflection_response::MessageResponse::ListServicesResponse(list)) => list
            .service
            .into_iter()
            .map(|s| s.name)
            .filter(|n| !n.starts_with("grpc.reflection."))
            .collect(),
        _ => return Err("the server's reflection answer has no services".into()),
    };
    let mut files: Vec<prost_types::FileDescriptorProto> = Vec::new();
    let mut wanted: Vec<v::server_reflection_request::MessageRequest> = names
        .iter()
        .map(|n| v::server_reflection_request::MessageRequest::FileContainingSymbol(n.clone()))
        .collect();
    while let Some(request) = wanted.pop() {
        let Some(v::server_reflection_response::MessageResponse::FileDescriptorResponse(found)) =
            ask(request).await?.message_response
        else {
            continue;
        };
        for bytes in found.file_descriptor_proto {
            let file = prost_types::FileDescriptorProto::decode(bytes.as_slice())
                .map_err(|e| e.to_string())?;
            if files.iter().any(|f| f.name == file.name) {
                continue;
            }
            for dependency in &file.dependency {
                if !files
                    .iter()
                    .any(|f| f.name.as_deref() == Some(dependency.as_str()))
                {
                    wanted.push(
                        v::server_reflection_request::MessageRequest::FileByFilename(
                            dependency.clone(),
                        ),
                    );
                }
            }
            files.push(file);
        }
    }
    pool_from(files)
}

/// A pool from files in any order: each is added once its dependencies are in.
fn pool_from(mut files: Vec<prost_types::FileDescriptorProto>) -> Result<DescriptorPool, String> {
    let mut pool = DescriptorPool::new();
    while !files.is_empty() {
        let before = files.len();
        let mut rest = Vec::new();
        for file in files {
            let ready = file
                .dependency
                .iter()
                .all(|d| pool.get_file_by_name(d).is_some());
            if ready {
                pool.add_file_descriptor_proto(file)
                    .map_err(|e| e.to_string())?;
            } else {
                rest.push(file);
            }
        }
        if rest.len() == before {
            let missing: Vec<String> = rest.iter().flat_map(|f| f.dependency.clone()).collect();
            return Err(format!(
                "the server didn't send these files: {}",
                missing.join(", ")
            ));
        }
        files = rest;
    }
    Ok(pool)
}

fn status_text(status: &Status) -> String {
    let message = status.message();
    format!(
        "{}{}",
        code_name(status.code()),
        if message.is_empty() {
            String::new()
        } else {
            format!(": {message}")
        }
    )
}

/// The services and methods in a pool.
pub fn services(pool: &DescriptorPool) -> Vec<ServiceInfo> {
    pool.services()
        .filter(|s| !s.full_name().starts_with("grpc.reflection."))
        .map(|s| ServiceInfo {
            name: s.full_name().to_string(),
            methods: s
                .methods()
                .map(|m| MethodInfo {
                    name: m.name().to_string(),
                    path: format!("{}/{}", s.full_name(), m.name()),
                    client_streaming: m.is_client_streaming(),
                    server_streaming: m.is_server_streaming(),
                    input: m.input().full_name().to_string(),
                    output: m.output().full_name().to_string(),
                    example: serde_json::to_string_pretty(&example(&m.input(), 0))
                        .unwrap_or_default(),
                })
                .collect(),
        })
        .collect()
}

/// A JSON value with every field of a message, filled with placeholders.
fn example(message: &MessageDescriptor, depth: usize) -> Value {
    let mut out = serde_json::Map::new();
    for field in message.fields() {
        let value = field_example(&field, depth);
        out.insert(field.json_name().to_string(), value);
    }
    Value::Object(out)
}

fn field_example(field: &FieldDescriptor, depth: usize) -> Value {
    let one = match field.kind() {
        Kind::Double | Kind::Float => json!(0.0),
        Kind::Int32 | Kind::Sint32 | Kind::Sfixed32 | Kind::Uint32 | Kind::Fixed32 => json!(0),
        // 64-bit numbers are strings in protobuf's JSON.
        Kind::Int64 | Kind::Sint64 | Kind::Sfixed64 | Kind::Uint64 | Kind::Fixed64 => json!("0"),
        Kind::Bool => json!(false),
        Kind::String => json!(""),
        Kind::Bytes => json!(""),
        Kind::Enum(e) => json!(
            e.values()
                .next()
                .map(|v| v.name().to_string())
                .unwrap_or_default()
        ),
        Kind::Message(m) if m.is_map_entry() => return json!({}),
        Kind::Message(m) => match m.full_name() {
            "google.protobuf.Timestamp" => json!("1970-01-01T00:00:00Z"),
            "google.protobuf.Duration" => json!("0s"),
            "google.protobuf.Struct" | "google.protobuf.Value" | "google.protobuf.Any" => json!({}),
            name if name.starts_with("google.protobuf.") && name.ends_with("Value") => Value::Null,
            _ if depth >= 3 => json!({}),
            _ => example(&m, depth + 1),
        },
    };
    if field.is_list() {
        Value::Array(vec![one])
    } else {
        one
    }
}

// Calling

/// A call in progress: send messages, then end the stream; events report what comes back.
pub struct Call {
    tx: Option<mpsc::UnboundedSender<DynamicMessage>>,
    task: tokio::task::AbortHandle,
    input: MessageDescriptor,
    events: EventSink,
    pub method: MethodDescriptor,
}

impl Call {
    /// Sends a JSON message. Fails if it doesn't fit the method's input type.
    pub fn send(&self, message: &str) -> Result<(), String> {
        let tx = self
            .tx
            .as_ref()
            .ok_or("the request stream is already closed")?;
        let mut deserializer = serde_json::Deserializer::from_str(message);
        let dynamic = DynamicMessage::deserialize(self.input.clone(), &mut deserializer)
            .map_err(|e| format!("the message doesn't fit {}: {e}", self.input.full_name()))?;
        let json = to_json(&dynamic);
        tx.send(dynamic)
            .map_err(|_| "the call has ended".to_string())?;
        (self.events)(GrpcEvent::Message {
            incoming: false,
            json,
            at: now_ms(),
        });
        Ok(())
    }

    /// Ends the request stream; the server answers and finishes.
    pub fn end(&mut self) {
        self.tx = None;
    }

    /// Stops the call at once, reporting it as cancelled.
    pub fn cancel(&mut self) {
        self.tx = None;
        if !self.task.is_finished() {
            self.task.abort();
            (self.events)(GrpcEvent::Done {
                code: Code::Cancelled as i32,
                status: code_name(Code::Cancelled).into(),
                message: "cancelled".into(),
                metadata: vec![],
                elapsed_ms: 0,
            });
        }
    }
}

fn to_json(message: &DynamicMessage) -> String {
    let options = SerializeOptions::new().skip_default_fields(false);
    let mut out = Vec::new();
    let mut serializer = serde_json::Serializer::pretty(&mut out);
    match message.serialize_with_options(&mut serializer, &options) {
        Ok(()) => String::from_utf8(out).unwrap_or_default(),
        Err(e) => format!("(could not show the message: {e})"),
    }
}

fn metadata_list(map: &tonic::metadata::MetadataMap) -> Vec<Header> {
    map.clone()
        .into_headers()
        .iter()
        .map(|(n, v)| Header {
            name: n.to_string(),
            value: String::from_utf8_lossy(v.as_bytes()).into_owned(),
        })
        .collect()
}

/// Starts a call to `method` (`package.Service/Method`). Messages are sent with [`Call::send`]; the call finishes
/// after [`Call::end`] (or at once for a unary call's single message) with a [`GrpcEvent::Done`].
pub async fn start(
    target: &Target,
    pool: &DescriptorPool,
    method: &str,
    events: EventSink,
) -> Result<Call, String> {
    let (service, name) = method.trim_matches('/').split_once('/').ok_or_else(|| {
        format!("\"{method}\" is not a method; write it as package.Service/Method")
    })?;
    let descriptor = pool
        .get_service_by_name(service)
        .ok_or_else(|| format!("there is no service {service}"))?
        .methods()
        .find(|m| m.name() == name)
        .ok_or_else(|| format!("{service} has no method {name}"))?;
    let path = ::http::uri::PathAndQuery::try_from(format!("/{service}/{name}"))
        .map_err(|e| e.to_string())?;
    let channel = target.channel().await?;
    let (tx, rx) = mpsc::unbounded_channel::<DynamicMessage>();
    let request = target.request(UnboundedReceiverStream::new(rx))?;
    let codec = DynamicCodec::new(descriptor.output());
    let sink = events.clone();
    let started = Instant::now();
    let task = tokio::spawn(async move {
        let mut grpc = tonic::client::Grpc::new(channel);
        let done = |status: &Status, trailers: Vec<Header>| GrpcEvent::Done {
            code: status.code() as i32,
            status: code_name(status.code()).into(),
            message: status.message().to_string(),
            metadata: trailers,
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
        if let Err(e) = grpc.ready().await {
            sink(done(&Status::unavailable(error_chain(&e)), vec![]));
            return;
        }
        let response = match grpc.streaming(request, path, codec).await {
            Ok(response) => response,
            Err(status) => {
                sink(done(&status, metadata_list(status.metadata())));
                return;
            }
        };
        sink(GrpcEvent::Headers {
            metadata: metadata_list(response.metadata()),
        });
        let mut stream = response.into_inner();
        loop {
            match stream.message().await {
                Ok(Some(message)) => sink(GrpcEvent::Message {
                    incoming: true,
                    json: to_json(&message),
                    at: now_ms(),
                }),
                Ok(None) => {
                    let trailers = stream
                        .trailers()
                        .await
                        .ok()
                        .flatten()
                        .map(|t| metadata_list(&t))
                        .unwrap_or_default();
                    sink(done(&Status::ok(""), trailers));
                    return;
                }
                Err(status) => {
                    sink(done(&status, metadata_list(status.metadata())));
                    return;
                }
            }
        }
    });
    Ok(Call {
        tx: Some(tx),
        task: task.abort_handle(),
        input: descriptor.input(),
        events,
        method: descriptor,
    })
}

/// The result of a whole call, as the runner needs it.
#[derive(Debug, Clone)]
pub struct Finished {
    pub code: Code,
    pub message: String,
    pub headers: Vec<Header>,
    pub trailers: Vec<Header>,
    /// The messages received, as JSON.
    pub messages: Vec<String>,
    pub elapsed_ms: u64,
}

/// Sends every message, ends the stream and waits for the end of the call.
pub async fn call(
    target: &Target,
    pool: &DescriptorPool,
    method: &str,
    messages: &[String],
) -> Result<Finished, String> {
    let (tx, mut rx) = mpsc::unbounded_channel::<GrpcEvent>();
    let sink: EventSink = Arc::new(move |e| {
        let _ = tx.send(e);
    });
    let mut call = start(target, pool, method, sink).await?;
    let single = !call.method.is_client_streaming();
    for (i, message) in messages.iter().enumerate() {
        if single && i > 0 {
            return Err(format!(
                "{method} takes one message, not {}",
                messages.len()
            ));
        }
        call.send(message)?;
    }
    if messages.is_empty() {
        call.send("{}")?;
    }
    call.end();
    drop(call);
    let mut finished = Finished {
        code: Code::Unknown,
        message: String::new(),
        headers: vec![],
        trailers: vec![],
        messages: vec![],
        elapsed_ms: 0,
    };
    while let Some(event) = rx.recv().await {
        match event {
            GrpcEvent::Headers { metadata } => finished.headers = metadata,
            GrpcEvent::Message {
                incoming: true,
                json,
                ..
            } => finished.messages.push(json),
            GrpcEvent::Message { .. } => {}
            GrpcEvent::Done {
                code,
                message,
                metadata,
                elapsed_ms,
                ..
            } => {
                finished.code = Code::from_i32(code);
                finished.message = message;
                finished.trailers = metadata;
                finished.elapsed_ms = elapsed_ms;
                return Ok(finished);
            }
        }
    }
    Err("the call ended without a status".into())
}

/// A finished call as an HTTP-like response, so checks, tests and scripts work on gRPC requests: the status is the
/// closest HTTP one (with `grpc-status` and `grpc-message` headers), and the body is the message, or a JSON array
/// when there are several.
pub fn as_response(url: &str, finished: &Finished) -> http::Response {
    let body = match finished.messages.as_slice() {
        [one] => one.clone(),
        many => {
            let values: Vec<Value> = many
                .iter()
                .filter_map(|m| serde_json::from_str(m).ok())
                .collect();
            serde_json::to_string_pretty(&values).unwrap_or_default()
        }
    };
    let mut headers = finished.headers.clone();
    headers.extend(finished.trailers.iter().cloned());
    headers.push(Header {
        name: "grpc-status".into(),
        value: (finished.code as i32).to_string(),
    });
    if !finished.message.is_empty() {
        headers.push(Header {
            name: "grpc-message".into(),
            value: finished.message.clone(),
        });
    }
    headers.push(Header {
        name: "content-type".into(),
        value: "application/json".into(),
    });
    let elapsed = std::time::Duration::from_millis(finished.elapsed_ms);
    http::Response {
        status: http_status(finished.code),
        reason: code_name(finished.code).into(),
        version: "HTTP/2".into(),
        url: url.to_string(),
        remote_addr: None,
        headers,
        encoded_size: body.len(),
        body: bytes::Bytes::from(body),
        timing: http::Timing {
            total: elapsed,
            ..Default::default()
        },
        redirects: vec![],
        sent: http::SentRequest {
            method: "POST".into(),
            url: url.to_string(),
            headers: vec![],
            body_size: 0,
        },
    }
}
