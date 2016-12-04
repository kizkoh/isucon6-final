#![feature(custom_derive)]
#![feature(custom_attribute)]

#[macro_use]
extern crate nickel;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate mysql;
extern crate chrono;
extern crate hyper;
extern crate serde;
extern crate serde_json;

// {Middleware, Action, StaticFilesHandler, FormBody};

use chrono::naive::datetime::NaiveDateTime;

use hyper::header::ContentType;
use hyper::mime::{Mime, TopLevel, SubLevel, Attr, Value};
use nickel::{Request, Response, MiddlewareResult};
use nickel::{Nickel, NickelError};
use nickel::Action;
use nickel::HttpRouter;
use nickel::status::StatusCode;

// use serde::de::Deserializer;
// use serde::ser::Serializer;

use std::env;
use std::error::Error;
use std::io::Write;
use std::mem;
use std::process::exit;

use std::str::FromStr;
use std::sync::Arc;

#[allow(unused_attributes)]
// #[derive(Debug, Serialize, Deserialize)]
struct Token {
    #[serde(rename="ID")]
    id: i64,
    #[serde(rename="CSRFToken")]
    csrf_token: String,
    #[serde(rename="CreatedAt")]
    created_at: NaiveDateTime,
}

lazy_static! {
    pub static ref DBX: Arc<mysql::Pool> = Arc::new(conn());
}

fn generate_error<'mw, T: AsRef<str> + ?Sized>(mut res: Response<'mw>, msg: &T) -> MiddlewareResult<'mw> {
    let err_msg: &str = unsafe { mem::transmute(msg.as_ref()) };

    // Implementation of Err struct
    #[allow(unused_attributes)]
    #[derive(Debug)]
    struct Err {
        // Error: String,
        #[serde(rename="Error")]
        error: &'static str,
    };
    impl serde::Serialize for Err {
        fn serialize<S>(&self, serializer: &mut S) -> Result<(), S::Error>
            where S: serde::Serializer
        {
            let mut err = try!(serializer.serialize_struct("Err", 1));
            try!(serializer.serialize_struct_elt(&mut err, "err", &self.error));
            serializer.serialize_struct_end(err)
        }
    }

    let err = Err { error: err_msg };
    let b = match serde_json::to_string(&err) {
        Ok(b) => b,
        Err(e) => {
            let _result = writeln!(&mut std::io::stderr(), "{}", msg.as_ref());
            let _result = writeln!(&mut std::io::stderr(), "{}", e.description());
            exit(1);
        }
    };

    // ToDo: delete the following comments
    // let content_type: mime::Mime = "application/json;charset=utf-8".parse().unwrap();
    // res.set(ContentType(content_type.into()));
    res.set(ContentType(Mime(TopLevel::Application,
                             SubLevel::Json,
                             vec![(Attr::Charset, Value::Utf8)])));
    let _result = writeln!(&mut std::io::stderr(), "{}", msg.as_ref());
    res.error(StatusCode::InternalServerError, b)
}

// func outputError(w http.ResponseWriter, err error) {

// 	b, _ := json.Marshal(struct {
// 		Error string `json:"error"`
// 	}{Error: "InternalServerError"})
// 	fmt.Fprintln(os.Stderr, err.Error())
// }

fn post_api_csrf_token<'mw>(_req: &mut Request, mut res: Response<'mw>) -> MiddlewareResult<'mw> {
    let mut query = "INSERT INTO `tokens` (`csrf_token`) VALUES".to_string();
    query = query + &"(SHA2(CONCAT(RAND(), UUID_SHORT()), 256))".to_string();

    let last_insert_id = match DBX.prep_exec(query.clone(), ()) {
        Ok(row) => row.last_insert_id(),
        Err(e) => return generate_error(res, e.description()),
    };

    query = "SELECT `id`, `csrf_token`, `created_at` FROM `tokens` WHERE id = :id".to_string();
    match DBX.first_exec(query.clone(), (params!{"id" => last_insert_id})) {
        Ok(option) => {
            match option {
                Some(mut row) => {
                    let csrf_token: String = row.take("csrf_token").unwrap();

                    // Implementation of Token struct
                    #[allow(unused_attributes)]
                    #[derive(Debug)]
                    struct Token {
                        #[serde(rename="Token")]
                        token: String,
                    };
                    impl serde::Serialize for Token {
                        fn serialize<S>(&self, serializer: &mut S) -> Result<(), S::Error>
                            where S: serde::Serializer
                        {
                            let mut token = try!(serializer.serialize_struct("Token", 1));
                            try!(serializer.serialize_struct_elt(&mut token, "token", &self.token));
                            serializer.serialize_struct_end(token)
                        }
                    }

                    let token = Token { token: csrf_token };
                    let b = match serde_json::to_string(&token) {
                        Ok(b) => b,
                        Err(e) => return generate_error(res, e.description()),
                    };
                    res.set(ContentType(Mime(TopLevel::Application,
                                             SubLevel::Json,
                                             vec![(Attr::Charset, Value::Utf8)])));
                    res.set(StatusCode::Ok);
                    return res.send(b);
                }
                None => return generate_error(res, "row is none"),
            };
        }
        Err(e) => return generate_error(res, e.description()),
    };

    // b, _ := json.Marshal(struct {
    // 	Token string `json:"token"`
    // }{Token: t.CSRFToken})
}

fn get_api_rooms<'mw>(_req: &mut Request, mut res: Response<'mw>) -> MiddlewareResult<'mw> {
    res.set(StatusCode::Ok);
    res.send("ok")
}

fn post_api_rooms<'mw>(_req: &mut Request, mut res: Response<'mw>) -> MiddlewareResult<'mw> {
    res.set(StatusCode::Ok);
    res.send("ok")
}

fn get_api_rooms_id<'mw>(_req: &mut Request, mut res: Response<'mw>) -> MiddlewareResult<'mw> {
    res.set(StatusCode::Ok);
    res.send("ok")
}

fn conn() -> mysql::Pool {
    let host = match env::var("MYSQL_HOST") {
        Ok(value) => value,
        Err(_) => "localhost".to_string(),
    };
    let port = match env::var("MYSQL_PORT") {
        Ok(value) => value,
        Err(_) => "3306".to_string(),
    };
    let port: u16 = match u16::from_str(&port) {
        Ok(value) => value,
        Err(e) => {
            println!("Failed to read DB port number from an environment variable MYSQL_PORT.\nError: {}",
                     e);
            exit(1);
        }
    };
    let user = match env::var("MYSQL_USER") {
        Ok(value) => value,
        Err(_) => "root".to_string(),
    };
    let password = match env::var("MYSQL_PASS") {
        Ok(value) => value,
        Err(_) => "".to_string(),
    };
    let dbname = "isuketch";

    let mut opts = mysql::OptsBuilder::new();
    opts.user(Some(user))
        .pass(Some(password))
        .ip_or_hostname(Some(host))
        .tcp_port(port)
        .db_name(Some(dbname))
        .init(vec!["charset=utf8mb4", "parseTime=true", "loc=Local"]); // 要検証
    // Use mysql::conn::new?
    match mysql::Pool::new(opts) {
        Ok(dbx) => dbx,
        Err(e) => {
            println!("Failed to connect to DB: {}.", e);
            exit(1);
        }
    }
}

fn main() {
    let mut server = Nickel::new();
    let mut router = Nickel::router();

    router.post("/api/csrf_token", post_api_csrf_token);
    router.get("/api/rooms", get_api_rooms);
    router.post("/api/rooms", post_api_rooms);
    router.get("/api/rooms/:id", get_api_rooms_id);

    server.utilize(router);

    // mux.HandleFuncC(pat.Get("/api/rooms/:id"), getAPIRoomsID)
    // mux.HandleFuncC(pat.Get("/api/stream/rooms/:id"), getAPIStreamRoomsID)
    // mux.HandleFuncC(pat.Post("/api/strokes/rooms/:id"), postAPIStrokesRoomsID)

    let custom_handler: fn(&mut NickelError<()>, &mut Request<()>) -> Action = custom_handler;
    server.handle_error(custom_handler);

    match server.listen("0.0.0.0:8888") {
        Ok(_) => (),
        Err(e) => {
            println!("{}", e);
            exit(1);
        }
    };
}

fn custom_handler<D>(err: &mut NickelError<D>, req: &mut Request<D>) -> Action {
    println!("|  - {} {} Error: {:?}",
             req.origin.method,
             req.origin.uri,
             err.message);
    Action::Continue(())
}
