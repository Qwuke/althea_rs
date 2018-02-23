use althea_types::{PaymentTx, Identity, LocalIdentity};

use actix::registry::SystemService;
use actix_web::*;
use actix_web::dev::*;

use futures::Future;

use payment_controller;
use payment_controller::PaymentController;

use tunnel_manager;
use tunnel_manager::TunnelManager;

use althea_kernel_interface::KernelInterface;

use std::sync::mpsc::Sender;
use std::sync::{Mutex, Arc};
use std::io::Read;
use std::boxed::Box;
use std::net::SocketAddr;

use serde_json;

use bytes::Bytes;

use settings::SETTING;

pub fn make_payments(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    trace!("Started processing payment from {:?}", req.connection_info().remote());

    req.body().from_err().and_then(move |bytes: Bytes| {
        println!("Payment body: {:?} from {:?}", bytes, req.connection_info().remote());
        let pmt: PaymentTx = serde_json::from_slice(&bytes[..]).unwrap();

        trace!("Received payment from {:?}, Payment: {:?}", pmt, req.connection_info().remote());
        PaymentController::from_registry().do_send(payment_controller::PaymentReceived(pmt));
        Ok(httpcodes::HTTPOk.into())
    }).responder()
}

pub fn hello_response(req: HttpRequest) -> Box<Future<Item=Json<LocalIdentity>, Error=Error>> {
    trace!("Started saying hello back to {:?}", req.connection_info().remote());

    let conn_info: SocketAddr = req.connection_info().remote().unwrap().parse().unwrap();

    req.body().from_err().and_then(move |bytes: Bytes| {
        println!("Hello body: {:?} from {:?}", bytes, req.connection_info().remote());
        let their_id: LocalIdentity = serde_json::from_slice(&bytes[..]).unwrap();

        trace!("Received neighbour identity, Payment: {:?}", their_id);

        let ki = KernelInterface{};

        TunnelManager::from_registry().send(tunnel_manager::GetLocalIdentity{
            requester: their_id.clone(),
        }).then(move |reply| {
            trace!("opening tunnel in hello_response for {:?}", their_id);
            TunnelManager::from_registry().do_send(tunnel_manager::OpenTunnel(their_id));
            Ok(Json(reply.unwrap()))
        })
    }).responder()
}