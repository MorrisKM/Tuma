//! The HTTP engine: build a [`Request`], [`send`] it, get a [`Response`] with a timing breakdown.

mod client;
mod error;
mod model;
mod prepare;
mod tls;

pub use client::send;
pub use error::Error;
pub use model::{
    ApiKeyLocation, Auth, Body, Header, KeyValue, Part, Redirect, Request, Response, SentRequest,
    Settings, Timing,
};
