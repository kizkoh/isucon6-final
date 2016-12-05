// How to run
// ISUCON_ENV=prodution MYSQL_HOST=localhost MYSQL_PORT=3306 MYSQL_USER=isucon MYSQL_PASS=isucon cargo run

// ISUCON_ENV: production // not need(?)
// MYSQL_HOST: mysql
// MYSQL_PORT: 3306
// MYSQL_USER: isucon
// MYSQL_PASS: isucon

// mysql -uisucon -pisucon -h localhost

#![feature(proc_macro)]
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
#[macro_use]
extern crate serde_derive;
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

// #[derive(Debug, Serialize, Deserialize)]
// struct Token {
//     #[serde(rename="ID")]
//     id: i64,
//     #[serde(rename="CSRFToken")]
//     csrf_token: String,
//     #[serde(rename="CreatedAt")]
//     created_at: NaiveDateTime,
// }

#[derive(Debug, Serialize, Deserialize)]
struct Point {
    id: i64,
    stroke_id: i64,
    x: f64,
    y: f64,
}

// type Point struct {
// 	ID       int64   `json:"id" db:"id"`
// 	StrokeID int64   `json:"stroke_id" db:"stroke_id"`
// 	X        float64 `json:"x" db:"x"`
// 	Y        float64 `json:"y" db:"y"`
// }

#[derive(Debug, Serialize, Deserialize)]
struct Stroke {
    id: i64,
    room_id: i64,
    width: i64,
    red: i64,
    green: i64,
    blue: i64,
    alpha: f64,
    // created_at: NaiveDateTime,
    created_at: String,
    points: Vec<Point>,
}
// type Stroke struct {
// 	ID        int64     `json:"id" db:"id"`
// 	RoomID    int64     `json:"room_id" db:"room_id"`
// 	Width     int       `json:"width" db:"width"`
// 	Red       int       `json:"red" db:"red"`
// 	Green     int       `json:"green" db:"green"`
// 	Blue      int       `json:"blue" db:"blue"`
// 	Alpha     float64   `json:"alpha" db:"alpha"`
// 	CreatedAt time.Time `json:"created_at" db:"created_at"`
// 	Points    []Point   `json:"points" db:"points"`
// }

#[derive(Debug, Serialize, Deserialize)]
struct Room {
    id: i64,
    name: String,
    canvas_width: i64,
    canvas_height: i64,
    // created_at: Vec<NaiveDateTime>,
    created_at: String,
    strokes: Vec<Stroke>,
    stroke_count: i64,
    watcher_count: i64,
}

// type Room struct {
// 	ID           int64     `json:"id" db:"id"`
// 	Name         string    `json:"name" db:"name"`
// 	CanvasWidth  int       `json:"canvas_width" db:"canvas_width"`
// 	CanvasHeight int       `json:"canvas_height" db:"canvas_height"`
// 	CreatedAt    time.Time `json:"created_at" db:"created_at"`
// 	Strokes      []Stroke  `json:"strokes"`
// 	StrokeCount  int       `json:"stroke_count"`
// 	WatcherCount int       `json:"watcher_count"`
// }

lazy_static! {
    pub static ref DBX: Arc<mysql::Pool> = Arc::new(conn());
}

fn get_strokes(room_id: i64, greather_than_id: i64) -> Result<Vec<Stroke>, mysql::Error> {
    let mut query = "SELECT `id`, `room_id`, `width`, `red`, `green`, `blue`, `alpha`, `created_at` FROM `strokes`"
        .to_string();
    query += " WHERE `room_id` = :room_id AND `id` > :greather_than_id ORDER BY `id` ASC";
    let mut strokes: Vec<Stroke> = vec![];
    match DBX.prep_exec(query.clone(),
                        (params!{"room_id" => room_id,
                                 "greather_than_id" => greather_than_id,
                        })) {
        Ok(rows) => {
            for row in rows {
                // FixMe: Use mysql::from_row()
                let mut row = row.unwrap();
                let created_at: NaiveDateTime = row.take("created_at").unwrap();
                let created_at = format!("{}", created_at.format("%Y-%m-%dT%H:%M:%S%.f%"));
                let stroke = Stroke {
                    id: row.take("id").unwrap(),
                    room_id: row.take("room_id").unwrap(),
                    width: row.take("width").unwrap(),
                    red: row.take("red").unwrap(),
                    green: row.take("green").unwrap(),
                    blue: row.take("blue").unwrap(),
                    alpha: row.take("alpha").unwrap(),
                    created_at: created_at,
                    points: vec![],
                };
                strokes.push(stroke);
            }
            Ok(strokes)
        }
        Err(e) => Err(e),
    }
}

fn get_room(room_id: i64) -> Result<Option<Room>, mysql::Error> {
    let query = "SELECT `id`, `name`, `canvas_width`, `canvas_height`, `created_at` FROM `rooms` WHERE `id` = :id";
    match DBX.first_exec(query.clone(), (params!{"id" => room_id})) {
        Ok(option) => {
            match option {
                Some(mut row) => {
                    let created_at: NaiveDateTime = row.take("created_at").unwrap();
                    let created_at = format!("{}", created_at.format("%Y-%m-%dT%H:%M:%S%.f%"));
                    let room = Room {
                        id: row.take("id").unwrap(),
                        name: row.take("name").unwrap(),
                        canvas_width: row.take("canvas_width").unwrap(),
                        canvas_height: row.take("canvas_height").unwrap(),
                        created_at: created_at,
                        strokes: vec![],
                        stroke_count: 0,
                        watcher_count: 0,
                    };
                    return Ok(Some(room));
                }
                None => return Ok(None),
            }
        }
        Err(e) => return Err(e),
    }
}

// fn get_watcher_count(room_id: i64) -> Result<i64, Error> {
// }

fn output_error_msg<'mw, T: AsRef<str> + ?Sized>(mut res: Response<'mw>,
                                                 status_code: StatusCode,
                                                 msg: &T)
    -> MiddlewareResult<'mw> {
    let err_msg: &str = unsafe { mem::transmute(msg.as_ref()) };

    // Implementation of Err struct
    #[derive(Debug, Deserialize)]
    struct Err {
        error: String,
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
    #[derive(Serialize, Deserialize)]
    enum Errize {
        #[serde(rename="error")]
        Err(String),
    }

    let err = Errize::Err(err_msg.to_string());
    let b = match serde_json::to_string(&err) {
        Ok(b) => b,
        Err(e) => {
            let _result = writeln!(&mut std::io::stderr(), "{}", msg.as_ref());
            let _result = writeln!(&mut std::io::stderr(), "{}", e.description());
            exit(1);
        }
    };

    // ToDo: Convert the following comments to rustdoc
    // let content_type: mime::Mime = "application/json;charset=utf-8".parse().unwrap();
    // res.set(ContentType(content_type.into()));
    res.set(ContentType(Mime(TopLevel::Application,
                             SubLevel::Json,
                             vec![(Attr::Charset, Value::Utf8)])));
    res.error(status_code, b)
}

fn output_error<'mw, T: AsRef<str> + ?Sized>(mut res: Response<'mw>, msg: &T) -> MiddlewareResult<'mw> {
    // Implementation of Err struct
    #[derive(Debug, Deserialize)]
    struct Err {
        error: String,
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
    #[derive(Serialize, Deserialize)]
    enum Errize {
        #[serde(rename="error")]
        Err(String),
    }

    let err = Errize::Err("InternalServerError".to_string());
    let b = match serde_json::to_string(&err) {
        Ok(b) => b,
        Err(e) => {
            let _result = writeln!(&mut std::io::stderr(), "{}", msg.as_ref());
            let _result = writeln!(&mut std::io::stderr(), "{}", e.description());
            exit(1);
        }
    };
    res.set(ContentType(Mime(TopLevel::Application,
                             SubLevel::Json,
                             vec![(Attr::Charset, Value::Utf8)])));
    let _result = writeln!(&mut std::io::stderr(), "{}", msg.as_ref());
    res.error(StatusCode::InternalServerError, b)
}

fn post_api_csrf_token<'mw>(_req: &mut Request, mut res: Response<'mw>) -> MiddlewareResult<'mw> {
    let mut query = "INSERT INTO `tokens` (`csrf_token`) VALUES".to_string();
    query = query + &"(SHA2(CONCAT(RAND(), UUID_SHORT()), 256))".to_string();

    let last_insert_id = match DBX.prep_exec(query.clone(), ()) {
        Ok(row) => row.last_insert_id(),
        Err(e) => return output_error(res, e.description()),
    };

    query = "SELECT `id`, `csrf_token`, `created_at` FROM `tokens` WHERE id = :id".to_string();
    match DBX.first_exec(query.clone(), (params!{"id" => last_insert_id})) {
        Ok(option) => {
            match option {
                Some(mut row) => {
                    let csrf_token: String = row.take("csrf_token").unwrap();

                    // Implementation of Token struct
                    #[derive(Debug, Deserialize)]
                    struct Token {
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
                    #[derive(Serialize, Deserialize)]
                    enum Tokenize {
                        Token(String),
                    }

                    // Use enum Tokenize::Token();
                    let token = Tokenize::Token(csrf_token);
                    let b = match serde_json::to_string(&token) {
                        Ok(b) => b,
                        Err(e) => return output_error(res, e.description()),
                    };
                    res.set(ContentType(Mime(TopLevel::Application,
                                             SubLevel::Json,
                                             vec![(Attr::Charset, Value::Utf8)])));
                    res.set(StatusCode::Ok);
                    return res.send(b);
                }
                None => return output_error(res, "row is none"),
            };
        }
        Err(e) => return output_error(res, e.description()),
    }
}

fn get_api_rooms<'mw>(_req: &mut Request, mut res: Response<'mw>) -> MiddlewareResult<'mw> {
    let mut query = "SELECT `room_id`, MAX(`id`) AS `max_id` FROM `strokes`".to_string();
    query += " GROUP BY `room_id` ORDER BY `max_id` DESC LIMIT 100";

    let mut rooms: Vec<Room> = vec![];

    match DBX.prep_exec(query.clone(), ()) {
        Ok(rows) => {
            for row in rows {
                let mut row = row.unwrap();
                let room_id: i64 = row.take("room_id").unwrap();
                let mut room = match get_room(room_id) {
                    Ok(room) => {
                        match room {
                            Some(room) => room,
                            None => return output_error(res, "row is none"),
                        }
                    }
                    Err(e) => return output_error(res, e.description()),
                };

                let s = match get_strokes(room.id, 0) {
                    Ok(s) => s,
                    Err(e) => return output_error(res, e.description()),
                };
                room.stroke_count = s.len() as i64;
                rooms.push(room);
            }
            // Implementation of Token struct
            #[derive(Debug, Deserialize)]
            struct RoomResp {
                rooms: Vec<Room>,
            };
            impl serde::Serialize for RoomResp {
                fn serialize<S>(&self, serializer: &mut S) -> Result<(), S::Error>
                    where S: serde::Serializer
                {
                    let mut room_resp = try!(serializer.serialize_struct("room_resp", 1));
                    try!(serializer.serialize_struct_elt(&mut room_resp, "rooms", &self.rooms));
                    serializer.serialize_struct_end(room_resp)
                }
            }
            #[derive(Serialize, Deserialize)]
            enum RoomRespize {
                RoomResp(Vec<Room>),
            }

            let room_resp = RoomRespize::RoomResp(rooms);
            let b = match serde_json::to_string(&room_resp) {
                Ok(b) => b,
                Err(e) => return output_error(res, e.description()),
            };

            res.set(ContentType(Mime(TopLevel::Application,
                                     SubLevel::Json,
                                     vec![(Attr::Charset, Value::Utf8)])));
            res.set(StatusCode::Ok);
            return res.send(b);
        }
        Err(e) => return output_error(res, e.description()),
    }
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
    // FixMe: Use mysql::conn::new?
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
