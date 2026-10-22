//! The HTTP engine: build a [`Request`], [`send`] it, get a [`Response`] with a timing breakdown.

pub mod auth;
mod client;
mod error;
mod model;
pub mod network;
mod prepare;
mod tls;

pub use client::{Observer, StreamEvent, send, send_observed, send_with};
pub use error::Error;
pub use model::{
    ApiKeyLocation, Auth, Body, Header, KeyValue, Part, Redirect, Request, Response, SentRequest,
    Settings, Timing,
};
pub use network::Network;
