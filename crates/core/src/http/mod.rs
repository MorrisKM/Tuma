//! The HTTP engine: build a [`Request`], [`send`] it, get a [`Response`] with a timing breakdown.

pub mod auth;
mod client;
mod error;
mod model;
pub mod network;
mod prepare;
pub(crate) mod tls;
pub mod websocket;

pub use client::{Observer, StreamEvent, send, send_observed, send_with};
pub(crate) use client::{connect_to, proxy_authorization, tunnel};
pub use error::Error;
pub use model::{
    ApiKeyLocation, Auth, Body, Header, KeyValue, Part, Redirect, Request, RequestTokenAuth,
    Response, SentRequest, Settings, Timing,
};
pub use network::Network;
pub(crate) use prepare::prepare;
