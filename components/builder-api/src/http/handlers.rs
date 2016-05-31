// Copyright:: Copyright (c) 2015-2016 Chef Software, Inc.
//
// The terms of the Evaluation Agreement (Bldr) between Chef Software Inc. and the party accessing
// this file ("Licensee") apply to Licensee's use of the Software until such time that the Software
// is made available under an open source license such as the Apache 2.0 License.

//! A collection of handlers for the HTTP server's router

use std::result;
use std::sync::{Arc, Mutex};

use bodyparser;
use hab_net::routing::Broker;
use iron::prelude::*;
use iron::status;
use iron::headers::{Authorization, Bearer};
use protobuf;
use protocol::jobsrv::{Job, JobCreate, JobGet};
use protocol::sessionsrv::{Session, SessionCreate, SessionGet};
use protocol::vault::{Origin, OriginCreate, OriginGet};
use protocol::net::{Msg, NetError, ErrCode};
use router::Router;
use rustc_serialize::json::{self, ToJson};
use zmq;

pub fn authenticate(req: &mut Request, ctx: &Arc<Mutex<zmq::Context>>) -> result::Result<Session, Response> {
    match req.headers.get::<Authorization<Bearer>>() {
        Some(&Authorization(Bearer { ref token })) => {
            let mut conn = Broker::connect(&ctx).unwrap();
            let mut request = SessionGet::new();
            request.set_token(token.to_string());
            conn.route(&request).unwrap();
            match conn.recv() {
                Ok(rep) => {
                    match rep.get_message_id() {
                        "Session" => {
                            let session = protobuf::parse_from_bytes(rep.get_body()).unwrap();
                            Ok(session)
                        }
                        "NetError" => Err(render_net_error(&rep)),
                        _ => unreachable!("unexpected msg: {:?}", rep),
                    }
                }
                Err(e) => {
                    error!("session get, err={:?}", e);
                    Err(Response::with(status::InternalServerError))
                }
            }
        }
        _ => Err(Response::with(status::Unauthorized)),
    }
}

pub fn session_create(req: &mut Request, ctx: &Arc<Mutex<zmq::Context>>) -> IronResult<Response> {
    let params = req.extensions.get::<Router>().unwrap();
    let code = match params.find("code") {
        Some(code) => code.to_string(),
        _ => return Ok(Response::with(status::BadRequest)),
    };
    let mut conn = Broker::connect(&ctx).unwrap();
    let mut request = SessionCreate::new();
    request.set_code(code.to_string());
    conn.route(&request).unwrap();
    match conn.recv() {
        Ok(rep) => {
            match rep.get_message_id() {
                "Session" => {
                    let token: Session = protobuf::parse_from_bytes(rep.get_body()).unwrap();
                    let encoded = json::encode(&token.to_json()).unwrap();
                    Ok(Response::with((status::Ok, encoded)))
                }
                "NetError" => Ok(render_net_error(&rep)),
                _ => unreachable!("unexpected msg: {:?}", rep),
            }
        }
        Err(e) => {
            error!("{:?}", e);
            Ok(Response::with(status::ServiceUnavailable))
        }
    }
}

pub fn origin_show(req: &mut Request, ctx: &Arc<Mutex<zmq::Context>>) -> IronResult<Response> {
    let params = req.extensions.get::<Router>().unwrap();
    let origin = match params.find("origin") {
        Some(origin) => origin.to_string(),
        _ => return Ok(Response::with(status::BadRequest)),
    };
    let mut conn = Broker::connect(&ctx).unwrap();
    let mut request = OriginGet::new();
    request.set_name(origin);
    conn.route(&request).unwrap();
    match conn.recv() {
        Ok(rep) => {
            match rep.get_message_id() {
                "Origin" => {
                    let origin: Origin = protobuf::parse_from_bytes(rep.get_body()).unwrap();
                    let encoded = json::encode(&origin.to_json()).unwrap();
                    Ok(Response::with((status::Ok, encoded)))
                }
                "NetError" => Ok(render_net_error(&rep)),
                _ => unreachable!("unexpected msg: {:?}", rep),
            }
        }
        Err(e) => {
            error!("{:?}", e);
            Ok(Response::with(status::ServiceUnavailable))
        }
    }
}

pub fn origin_create(req: &mut Request, ctx: &Arc<Mutex<zmq::Context>>) -> IronResult<Response> {
    let session = match authenticate(req, ctx) {
        Ok(session) => session,
        Err(response) => return Ok(response),
    };
    let mut request = OriginCreate::new();
    request.set_owner_id(session.get_id());
    match req.get::<bodyparser::Json>() {
        Ok(Some(body)) => {
            match body.find("name") {
                Some(origin) => request.set_name(origin.as_string().unwrap().to_owned()),
                _ => return Ok(Response::with(status::BadRequest)),
            }
        }
        _ => return Ok(Response::with(status::BadRequest)),
    };
    let mut conn = Broker::connect(&ctx).unwrap();
    conn.route(&request).unwrap();
    match conn.recv() {
        Ok(rep) => {
            match rep.get_message_id() {
                "Origin" => {
                    let origin: Origin = protobuf::parse_from_bytes(rep.get_body()).unwrap();
                    let encoded = json::encode(&origin.to_json()).unwrap();
                    Ok(Response::with((status::Created, encoded)))
                }
                "NetError" => Ok(render_net_error(&rep)),
                _ => unreachable!("unexpected msg: {:?}", rep),
            }
        }
        Err(e) => {
            error!("{:?}", e);
            Ok(Response::with(status::ServiceUnavailable))
        }
    }
}

pub fn job_create(req: &mut Request, ctx: &Arc<Mutex<zmq::Context>>) -> IronResult<Response> {
    let session = match authenticate(req, ctx) {
        Ok(session) => session,
        Err(response) => return Ok(response),
    };
    let mut conn = Broker::connect(&ctx).unwrap();
    let mut request = JobCreate::new();
    request.set_owner_id(session.get_id());
    conn.route(&request).unwrap();
    match conn.recv() {
        Ok(rep) => {
            match rep.get_message_id() {
                "Job" => {
                    let job: Job = protobuf::parse_from_bytes(rep.get_body()).unwrap();
                    let encoded = json::encode(&job.to_json()).unwrap();
                    Ok(Response::with((status::Created, encoded)))
                }
                "NetError" => Ok(render_net_error(&rep)),
                _ => unreachable!("unexpected msg: {:?}", rep),
            }
        }
        Err(e) => {
            error!("{:?}", e);
            Ok(Response::with(status::ServiceUnavailable))
        }
    }
}

pub fn job_show(req: &mut Request, ctx: &Arc<Mutex<zmq::Context>>) -> IronResult<Response> {
    let params = req.extensions.get::<Router>().unwrap();
    let id = match params.find("id") {
        Some(id) => {
            match id.parse() {
                Ok(id) => id,
                Err(_) => return Ok(Response::with(status::BadRequest)),
            }
        }
        _ => return Ok(Response::with(status::BadRequest)),
    };
    let mut conn = Broker::connect(&ctx).unwrap();
    let mut request = JobGet::new();
    request.set_id(id);
    conn.route(&request).unwrap();
    match conn.recv() {
        Ok(rep) => {
            match rep.get_message_id() {
                "Job" => {
                    let job: Job = protobuf::parse_from_bytes(rep.get_body()).unwrap();
                    let encoded = json::encode(&job.to_json()).unwrap();
                    Ok(Response::with((status::Ok, encoded)))
                }
                "NetError" => Ok(render_net_error(&rep)),
                _ => unreachable!("unexpected msg: {:?}", rep),
            }
        }
        Err(e) => {
            error!("{:?}", e);
            Ok(Response::with(status::ServiceUnavailable))
        }
    }
}

/// Return an IronResult containing the body of a NetError and the appropriate HTTP response status
/// for the corresponding NetError.
///
/// For example, a NetError::ENTITY_NOT_FOUND will result in an HTTP response containing the body
/// of the NetError with an HTTP status of 404.
///
/// # Panics
///
/// * The given encoded message was not a NetError
/// * The given messsage could not be decoded
/// * The NetError could not be encoded to JSON
fn render_net_error(msg: &Msg) -> Response {
    assert_eq!(msg.get_message_id(), "NetError");
    let err: NetError = protobuf::parse_from_bytes(msg.get_body()).unwrap();
    let encoded = json::encode(&err.to_json()).unwrap();
    let status = match err.get_code() {
        ErrCode::ENTITY_NOT_FOUND => status::NotFound,
        ErrCode::NO_SHARD => status::ServiceUnavailable,
        ErrCode::TIMEOUT => status::RequestTimeout,
        _ => status::InternalServerError,
    };
    Response::with((status, encoded))
}