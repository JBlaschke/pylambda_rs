use std::net::SocketAddr;
use std::u64;

use http_body_util::{Empty, Full};
use hyper::body::{Body, Bytes};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use serde::{Deserialize, Serialize};

use hyper::{Method, StatusCode};
use http_body_util::{combinators::BoxBody, BodyExt};

use serde_json;
use pyo3::prelude::*;
use pyo3::types::{PyDict, IntoPyDict};

use pyo3::ffi::c_str;
use std::ffi::CStr;
use std::ffi::CString;


#[derive(Serialize, Deserialize)]
struct PythonRequest {
    code: String,
}


fn empty() -> BoxBody<Bytes, hyper::Error> {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}


fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}


fn exec_python_cmd(python_request: &PythonRequest) -> Result<String, PyErr> {
    let mut result: Result<String, PyErr> = Ok(String::from(""));

    Python::with_gil(|py| {
        let code = &python_request.code;

        // Simple framework to capture output by redirecting stdout and stderr
        let cmd: String = format!(
"import sys, json
from io import StringIO
class Capturing():
    def __init__(self, buffer):
        self.buffer = buffer
    def __enter__(self):
        self.old_stdout = sys.stdout
        self.old_stderr = sys.stderr
        sys.stdout = StringIO()
        sys.stderr = StringIO()
        return self
    def __exit__(self, *args):
        self.buffer['stdout'] = sys.stdout.getvalue()
        self.buffer['stderr'] = sys.stderr.getvalue()
        sys.stdout = self.old_stdout
        sys.stderr = self.old_stderr

def run():
    results = dict()
    results['io_buffer'] = dict()
    with Capturing(results['io_buffer']) as output:    
        {}
    return json.dumps(results)",
            code
        );

        println!("-------- executing python command: ----------");
        println!("{:}", cmd);
        println!("------------------ done ---------------------");

        let cmd_c_string: CString = CString::new(cmd.as_str()).unwrap();
        let cmd_c_str: &CStr = cmd_c_string.as_c_str();

        let cmd_capture_io = PyModule::from_code(
            py,
            cmd_c_str,
            c_str!("stdio_captures.py"),
            c_str!("stdio_captures")
        ).unwrap();

        let runner = cmd_capture_io.getattr("run").unwrap();
        result = match runner.call0() {
            Ok(r) => {
                // Retrieve the output from the buffer
                println!("{:?}", r);
                let output = r.to_string();
                Ok(output)
            },
            Err(e) => {
                // Error during execution
                Err(e)
            }
        };
    });

    result
}


// fn incoming_to_string(body: hyper::body::Incoming) -> Result<Vec<u8>, hyper::Error> {
//     let mut bytes: Vec<u8> = Vec::new();
//     for chunk in body.chunks()? {
//         bytes.extend_from_slice(&chunk.to_vec()?);
//     }
// 
//     Ok(bytes)
// }


async fn run_python_code(
    req: Request<hyper::body::Incoming>
)-> Result<Response<String>, hyper::Error> {

    match (req.method(), req.uri().path()) {
        (&Method::POST, "/run") => {

            //let body = req.into_body().concat2().wait().unwrap().into_bytes();
            // let body = incoming_to_string(req.into_body())?;

            let body = req.collect().await?.to_bytes();

            let python_request: PythonRequest = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => {
                    return Ok(
                        Response::builder()
                        .status(400)
                        .body(String::from("Invalid JSON"))
                        .unwrap()
                    )
                }
            };

            match exec_python_cmd(&python_request) {
                Ok(out) => {
                    Ok(Response::builder()
                        .status(200)
                        .body(String::from(out))
                        .unwrap())
                },
                Err(e) => {
                    let error_msg = format!("Error: {:?}", e);
                    Ok(Response::builder()
                        .status(500)
                        .body(String::from(error_msg))
                        .unwrap())
                }
            }
        },

        _ => {
            Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(String::from("Not found"))
                .unwrap())
        }
    }
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));

    let listener = TcpListener::bind(addr).await?;

    loop {
        let (stream, _) = listener.accept().await?;

        let io = TokioIo::new(stream);

        tokio::task::spawn(async move {
            let builder = http1::Builder::new();
            let service = builder.serve_connection(io, service_fn(run_python_code));

            if let Err(err) = service.await {
                eprintln!("Error serving connection: {:?}", err);
            }
        });
    }
}
