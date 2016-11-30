// How to run
// ISUCON_ENV=prodution MYSQL_HOST=localhost MYSQL_PORT=3306 MYSQL_USER=isucon MYSQL_PASS=isucon cargo run

// ISUCON_ENV: production // not need(?)
// MYSQL_HOST: mysql
// MYSQL_PORT: 3306
// MYSQL_USER: isucon
// MYSQL_PASS: isucon

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

use chrono::naive::datetime::NaiveDateTime;
use chrono::datetime::DateTime;
use chrono::offset::Offset;
use std::ops::Sub;

use hyper::header::ContentType;
use hyper::mime::{Mime, TopLevel, SubLevel, Attr, Value};
use hyper::net::Streaming;
use nickel::{Request, Response, MiddlewareResult};
use nickel::{Nickel, NickelError, QueryString};
use nickel::Action;
use nickel::HttpRouter;
use nickel::status::StatusCode;
use mysql::conn::IsolationLevel;

use std::env;
use std::error::Error;
use std::io::{Read, Write};
use std::mem;
use std::process::exit;
use std::str::FromStr;
use std::sync::Arc;
use std::thread;
use std::time;

#[derive(Debug, Serialize, Deserialize)]
struct Token {
    id: i64,
    csrf_token: String,
    // Serde do not implement NativeDateTime serializer, deserializer
    // created_at: NaiveDateTime,
    created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Point {
    #[serde(default = "default_i64")]
    id: i64,
    #[serde(default = "default_i64")]
    stroke_id: i64,
    x: f64,
    y: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Stroke {
    id: i64,
    #[serde(default = "default_i64")]
    room_id: i64,
    width: i64,
    red: i64,
    green: i64,
    blue: i64,
    alpha: f64,
    // Serde do not implement NativeDateTime serializer, deserializer
    // created_at: NaiveDateTime,
    #[serde(default = "default_string")]
    created_at: String,
    #[serde(default = "default_point_vec")]
    points: Vec<Point>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Room {
    #[serde(default = "default_i64")]
    id: i64,
    name: String,
    canvas_width: i64,
    canvas_height: i64,
    // Serde do not implement NativeDateTime serializer, deserializer
    // created_at: NaiveDateTime,
    #[serde(default = "default_string")]
    created_at: String,
    #[serde(default = "default_stroke_vec")]
    strokes: Vec<Stroke>,
    #[serde(default = "default_i64")]
    stroke_count: i64,
    #[serde(default = "default_i64")]
    watcher_count: i64,
}

fn default_stroke_vec() -> Vec<Stroke> {
    vec![]
}

fn default_point_vec() -> Vec<Point> {
    vec![]
}

fn default_string() -> String {
    "".to_string()
}

fn default_i64() -> i64 {
    0
}

lazy_static! {
    pub static ref DBX: Arc<mysql::Pool> = Arc::new(conn());
}

fn print_and_flush<'mw, T: AsRef<str>>(mut streaming: Response<'mw, (), Streaming>, msg: T) -> Result<Response<'mw, (), Streaming>, MiddlewareResult<'mw>> {
    match streaming.write(msg.as_ref().as_bytes()) {
        Ok(_) => (),
        Err(e) => {
            let _result = writeln!(&mut std::io::stderr(), "{}", e.description());
            return Err(streaming.bail(e.description().to_string()))
        }
    }
    match streaming.flush() {
        Ok(_) => Ok(streaming),
        Err(e) => {
            #[derive(Debug, Serialize)]
            #[serde(rename="error")]
            struct Error {
                error: String,
            };

            let err = Error{ error: "InternalServerError".to_string() };
            let b = match serde_json::to_string(&err) {
                Ok(b) => b,
                Err(e) => {
                    // FixMe: return error as response
                    let _result = writeln!(&mut std::io::stderr(), "{}", e.description());
                    exit(1);
                }
            };
            let _result = writeln!(&mut std::io::stderr(), "{}", e.description());
            return Err(streaming.bail(b))
        },
    }
}

fn check_token(csrf_token: String) -> Result<Option<Token>, mysql::Error> {
    if csrf_token == "" {
        return Ok(None);
    }

    let mut query = "SELECT `id`, `csrf_token`, `created_at` FROM `tokens`".to_string();
    query += " WHERE `csrf_token` = :csrf_token AND `created_at` > CURRENT_TIMESTAMP(6) - INTERVAL 1 DAY";

    match DBX.first_exec(query.clone(),
                        params!{"csrf_token" => csrf_token,
                        }) {
        Ok(option) => {
            match option {
                Some(mut row) => {
                    let created_at: NaiveDateTime = row.take("created_at").unwrap();
                    let created_at: DateTime<chrono::Local> = DateTime::from_utc(created_at,
                                                                                 *chrono::Local::now().offset());
                    let created_at = created_at.sub(chrono::Local::now().offset().local_minus_utc());
                    let created_at = format!("{}", created_at.format("%Y-%m-%dT%H:%M:%S%.6f%:z"));
                    // FixMe: Use mysql::from_row()
                    let token = Token {
                        id: row.take("id").unwrap(),
                        csrf_token: row.take("csrf_token").unwrap(),
                        created_at: created_at,
                    };
                    return Ok(Some(token));
                }
                None => return Ok(None),
            }
        }
        Err(e) => return Err(e),
    }
}

fn get_stroke_points(stroke_id: i64) -> Result<Vec<Point>, mysql::Error> {
    let query = "SELECT `id`, `stroke_id`, `x`, `y` FROM `points` WHERE `stroke_id` = :stroke_id ORDER BY `id` ASC".to_string();
    let mut ps: Vec<Point> = vec![];
    match DBX.prep_exec(query.clone(),
                        params!{"stroke_id" => stroke_id,
                        }) {
        Ok(rows) => {
            for row in rows {
                match row {
                    Ok(mut row) => {
                        let point = Point {
                            id: row.take("id").unwrap(),
                            stroke_id: row.take("stroke_id").unwrap(),
                            x: row.take("x").unwrap(),
                            y: row.take("y").unwrap(),
                        };
                        ps.push(point);
                    },
                    Err(e) => return Err(e),
                }
            }
            return Ok(ps);
        }
        Err(e) => return Err(e),
    }
}

fn get_strokes(room_id: i64, greater_than_id: i64) -> Result<Vec<Stroke>, mysql::Error> {
    let mut query = "SELECT `id`, `room_id`, `width`, `red`, `green`, `blue`, `alpha`, `created_at` FROM `strokes`".to_string();
    query += " WHERE `room_id` = :room_id AND `id` > :greater_than_id ORDER BY `id` ASC";
    let mut strokes: Vec<Stroke> = vec![];
    match DBX.prep_exec(query.clone(),
                        params!{"room_id" => room_id,
                                 "greater_than_id" => greater_than_id,
                        }) {
        Ok(rows) => {
            for row in rows {
                // FixMe: Use mysql::from_row()
                let mut row = row.unwrap();
                    let created_at: NaiveDateTime = row.take("created_at").unwrap();
                    let created_at: DateTime<chrono::Local> = DateTime::from_utc(created_at,
                                                                                 *chrono::Local::now().offset());
                    let created_at = created_at.sub(chrono::Local::now().offset().local_minus_utc());
                    let created_at = format!("{}", created_at.format("%Y-%m-%dT%H:%M:%S%.6f%:z"));
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
                    let created_at: DateTime<chrono::Local> = DateTime::from_utc(created_at,
                                                                                 *chrono::Local::now().offset());
                    let created_at = created_at.sub(chrono::Local::now().offset().local_minus_utc());
                    let created_at = format!("{}", created_at.format("%Y-%m-%dT%H:%M:%S%.6f%:z"));
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

fn get_watcher_count(room_id: i64) -> Result<i64, mysql::Error> {
    let mut query = "SELECT COUNT(*) AS `watcher_count` FROM `room_watchers`".to_string();
    query += " WHERE `room_id` = :room_id AND `updated_at` > CURRENT_TIMESTAMP(6) - INTERVAL 3 SECOND";
    match DBX.first_exec(query.clone(),
                        params!{"room_id" => room_id,
                        }) {
        Ok(row) => {
            match row {
                Some(mut row) => {
                    Ok(row.take("watcher_count").unwrap())
                },
                None => Ok(0),
            }
        },
        Err(e) => return Err(e),
    }
}

fn update_room_watcher(room_id: i64, token_id: i64) -> Result<(), mysql::Error> {
    let mut query = "INSERT INTO `room_watchers` (`room_id`, `token_id`) VALUES (:room_id, :token_id)".to_string();
    query += " ON DUPLICATE KEY UPDATE `updated_at` = CURRENT_TIMESTAMP(6)";

    return match DBX.prep_exec(query.clone(),
                        params!{"room_id" => room_id,
                                "token_id" => token_id
                        }) {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}

fn output_error_msg<'mw, T: AsRef<str> + ?Sized>(mut res: Response<'mw>,
                                                 status_code: StatusCode,
                                                 msg: &T)
    -> MiddlewareResult<'mw> {
    let err_msg: &str = unsafe { mem::transmute(msg.as_ref()) };

    #[derive(Debug, Serialize)]
    #[serde(rename="error")]
    struct Error {
        error: String,
    };

    let err = Error{ error: err_msg.to_string() };
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
    res.set(status_code);
    res.send(b)
}

fn output_error<'mw, T: AsRef<str> + ?Sized>(mut res: Response<'mw>, msg: &T) -> MiddlewareResult<'mw> {
    #[derive(Debug, Serialize)]
    #[serde(rename="error")]
    struct Error {
        error: String,
    };

    let err = Error{ error: "InternalServerError".to_string() };
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

                    #[derive(Debug, Serialize)]
                    #[serde(rename="token")]
                    struct Token {
                        token: String,
                    };

                    let token = Token { token: csrf_token };
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

            #[derive(Debug, Serialize)]
            #[serde(rename="room")]
            struct RoomResp {
                rooms: Vec<Room>,
            };

            let room_resp = RoomResp {
                rooms: rooms,
            };
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

fn post_api_rooms<'mw>(req: &mut Request, mut res: Response<'mw>) -> MiddlewareResult<'mw> {
    let token: String;
    {
        let ref headers = req.origin.headers;
        // FixMe: implement error handling
        let csrf_token = headers.get_raw("x-csrf-token").unwrap().first().unwrap();
        token = String::from_utf8(csrf_token.clone()).unwrap();
    }
    let t = match check_token(token) {
        Ok(t) => match t {
            Some(t) => t,
            None => return output_error_msg(res, StatusCode::BadRequest, "トークンエラー。ページを再読み込みしてください。")
        },
        Err(e) => return output_error(res, e.description()),
    };
    let mut data = vec![];
    {
        match req.origin.read_to_end(&mut data) {
            Ok(_) => (),
            Err(e) => return output_error(res, e.description()),
        }
    }
    let body = String::from_utf8(data).unwrap();
    let posted_room: Room = match serde_json::from_str(body.as_ref()) {
        Ok(room) => room,
        Err(e) => return output_error(res, e.description()),
    };
    if posted_room.name == "" || posted_room.canvas_width == 0 || posted_room.canvas_height == 0 {
        return output_error_msg(res, StatusCode::BadRequest, "リクエストが正しくありません。");
    }
    let mut tx = match DBX.start_transaction(
        true,
        Some(IsolationLevel::ReadCommitted),
        Some(false)
    ) {
        Ok(tx) => tx,
        Err(e) => return output_error(res, e.description()),
    };

    let mut query = "INSERT INTO `rooms` (`name`, `canvas_width`, `canvas_height`)".to_string();
    query += " VALUES (:name, :canvas_width, :canvas_height)";
    let room_id = match tx.prep_exec(query.clone(),
                       params!{"name" => posted_room.name,
                               "canvas_width" => posted_room.canvas_width,
                               "canvas_height" => posted_room.canvas_height,
                       }) {
        Ok(row) => row.last_insert_id(),
        Err(e) => return output_error(res, e.description()),
    };

    query = "INSERT INTO `room_owners` (`room_id`, `token_id`) VALUES (:room_id, :token_id)".to_string();
    match tx.prep_exec(query.clone(),
                 params!{
                     "room_id" => room_id,
                     "token_id" => t.id,
                 }) {
        Ok(_) => (),
        Err(e) => return output_error(res, e.description()),
    }

    match tx.commit() {
        Ok(_) => (),
        Err(e) => {
            // FixMe: implement rollback
            // tx.rollback();
            return output_error(res, e.description());
        },
    }

    let room = match get_room(room_id as i64) {
        Ok(room) =>
            match room {
                Some(room) => {
                    room
                },
                None => return output_error(res, "row is none"),
            },
        Err(e) => return output_error(res, e.description()),
    };

    #[derive(Debug, Serialize)]
    #[serde(rename="room")]
    struct RoomResp {
        room: Room,
    };

    let room_resp = RoomResp{ room: room };
    let b = match serde_json::to_string(&room_resp) {
        Ok(b) => b,
        Err(e) => return output_error(res, e.description()),
    };

    res.set(ContentType(Mime(TopLevel::Application,
                             SubLevel::Json,
                             vec![(Attr::Charset, Value::Utf8)])));
    res.set(StatusCode::Ok);
    res.send(b)
}

fn get_api_rooms_id<'mw>(req: &mut Request, mut res: Response<'mw>) -> MiddlewareResult<'mw> {
    let id_str = match req.param("id") {
        Some(id) => id,
        None => "",
    };
    let id: i64 = match i64::from_str(id_str) {
        Ok(id) => id,
        Err(_) => return output_error_msg(res, StatusCode::NotFound, "この部屋は存在しません。"),
    };

    let mut room = match get_room(id) {
        Ok(room) =>
            match room {
                Some(room) => {
                    room
                },
                None => return output_error_msg(res, StatusCode::NotFound, "この部屋は存在しません。"),
            },
        Err(e) => return output_error(res, e.description()),
    };

    let mut strokes = match get_strokes(room.id, 0) {
        Ok(strokes) => strokes,
        Err(e) => return output_error(res, e.description()),
    };

    for i in 0..strokes.len() {
        let p = match get_stroke_points(strokes[i].id) {
            Ok(p) => p,
            Err(e) => return output_error(res, e.description()),
        };
        strokes[i].points = p;
    }

    room.strokes = strokes;
    room.watcher_count = match get_watcher_count(room.id) {
        Ok(watcher_count) => watcher_count,
        Err(e) => return output_error(res, e.description()),
    };

    #[derive(Debug, Serialize)]
    #[serde(rename="room")]
    struct RoomResp {
        room: Room,
    };
    let room_resp = RoomResp {
        room: room
    };
    let b = match serde_json::to_string(&room_resp) {
        Ok(b) => b,
        Err(e) => return output_error(res, e.description()),
    };

    res.set(ContentType(Mime(TopLevel::Application,
                             SubLevel::Json,
                             vec![(Attr::Charset, Value::Utf8)])));
    res.set(StatusCode::Ok);
    res.send(b)
}

fn get_api_stream_rooms_id<'mw>(req: &mut Request, mut res: Response<'mw>) -> MiddlewareResult<'mw> {
    res.set(ContentType(Mime(TopLevel::Text,
                             SubLevel::EventStream,
                             vec![])));
    let csrf_token = match req.query().get("csrf_token") {
            Some(csrf_token) => csrf_token.to_string(),
            None => "".to_string(),
    };

    let id_str = match req.param("id") {
        Some(id) => id,
        None => "",
    };
    let id: i64 = match i64::from_str(id_str) {
        Ok(id) => id,
        Err(_) => return output_error_msg(res, StatusCode::NotFound, "この部屋は存在しません。"),
    };

    let t = match check_token(csrf_token) {
        Ok(token) => match token {
            Some(token) => token,
            None => {
                let mut streaming = match res.start() {
                    Ok(streaming) => streaming,
                    Err(_e) => {
                        let _result = writeln!(&mut std::io::stderr(), "nickel streaming start error");
                        exit(1);
                    }
                };
                streaming = match print_and_flush(streaming, "event:bad_request\ndata:トークンエラー。ページを再読み込みしてください。\n\n") {
                    Ok(streaming) => streaming,
                    Err(e) => return e,
                };
                return Ok(Action::Halt(streaming))
            },
        },
        Err(e) => return output_error(res, e.description()),
    };

    let room = match get_room(id) {
        Ok(room) => match room {
            Some(room) => room,
            None => {
                let mut streaming = match res.start() {
                    Ok(streaming) => streaming,
                    Err(_e) => {
                        let _result = writeln!(&mut std::io::stderr(), "nickel streaming start error");
                        exit(1);
                    }
                };
                streaming = match print_and_flush(streaming, "event:bad_request\ndata:この部屋は存在しません\n\n") {
                    Ok(streaming) => streaming,
                    Err(e) => return e,
                };
                return Ok(Action::Halt(streaming))
            },
        },
        Err(e) => return output_error(res, e.description()),
    };

    match update_room_watcher(room.id, t.id) {
        Ok(_) => (),
        Err(e) => return output_error(res, e.description()),
    }

    let mut watcher_count = match get_watcher_count(room.id) {
        Ok(watcher_count) => watcher_count,
        Err(e) => return output_error(res,e .description()),
    };

    let mut streaming = match res.start() {
        Ok(streaming) => streaming,
        Err(_e) => {
            let _result = writeln!(&mut std::io::stderr(), "nickel streaming start error");
            exit(1);
        }
    };

    let msg = format!("retry:500\n\nevent:watcher_count\ndata:{}\n\n",
                      watcher_count);
    streaming = print_and_flush(streaming, msg).ok().unwrap();

    let mut last_stroke_id: i64;
    {
        let ref headers = req.origin.headers;
        last_stroke_id = match headers.get_raw("last-event-id") {
            Some(last_event_id_str_vec) => {
                let last_event_id_str = String::from_utf8(last_event_id_str_vec.first().unwrap().clone()).unwrap();
                match i64::from_str(last_event_id_str.as_ref()) {
                    Ok(id) => id,
                    // FixMe: return error as response
                    Err(_e) => {
                        let _result = writeln!(&mut std::io::stderr(), "getting last event id error");
                        exit(1);
                    },
                }
            },
            None => {
                0
            },
        }
    }

    for _i in 0..6 {
        thread::sleep(time::Duration::from_millis(500));
        let strokes = match get_strokes(room.id, last_stroke_id) {
            Ok(strokes) => strokes,
            // FixMe: return error as response
            Err(_e) => {
                let _result = writeln!(&mut std::io::stderr(), "getting last event id error");
                exit(1);
            }
        };
        for mut stroke in strokes {
            stroke.points = match get_stroke_points(stroke.id) {
                Ok(points) => points,
                // FixMe: return error as response
                Err(_e) => {
                    let _result = writeln!(&mut std::io::stderr(), "getting last event id error");
                    exit(1);
                }
            };
            let d = serde_json::ser::to_string(&stroke).unwrap();
            streaming = print_and_flush(streaming, format!("id:{}\n\nevent:stroke\ndata:{}\n\n",
                                                           stroke.id,
                                                           d)
            ).ok().unwrap();
            last_stroke_id = stroke.id;
        }

        match update_room_watcher(room.id, t.id) {
            Ok(_) => (),
            // FixMe: return error as response
            Err(_e) => {
                let _result = writeln!(&mut std::io::stderr(), "getting last event id error");
                exit(1);
            },
        }

        let new_watcher_count = match get_watcher_count(room.id) {
            Ok(watcher_count) => watcher_count,
            // FixMe: return error as response
            Err(_e) => {
                let _result = writeln!(&mut std::io::stderr(), "getting last event id error");
                exit(1);
            },
        };

        if new_watcher_count != watcher_count {
            watcher_count = new_watcher_count;
            streaming = print_and_flush(streaming, format!("event:watcher_count\ndata:{}\n\n", watcher_count)).ok().unwrap();
        }
    }
    streaming.bail("")
    // Ok(Action::Halt(streaming))
}

fn post_api_strokes_rooms_id<'mw>(req: &mut Request, mut res: Response<'mw>) -> MiddlewareResult<'mw> {
    let token: Token;
    {
        let ref headers = req.origin.headers;
        let csrf_token = match headers.get_raw("x-csrf-token") {
            Some(csrf_token) => {
                match csrf_token.first() {
                    Some(csrf_token) => String::from_utf8(csrf_token.clone()).unwrap(),
                    None => return output_error_msg(res, StatusCode::BadRequest, "トークンエラー。ページを再読み込みしてください。")
                }
            },
            None => return output_error_msg(res, StatusCode::BadRequest, "トークンエラー。ページを再読み込みしてください。")
        };
        token = match check_token(csrf_token) {
            Ok(token) => match token {
                Some(token) => token,
                None => return output_error_msg(res, StatusCode::BadRequest, "トークンエラー。ページを再読み込みしてください。")
            },
            Err(e) => return output_error(res, e.description()),
        };
    }

    let id: i64;
    {
        let id_str = match req.param("id") {
            Some(id) => id,
            None => "",
        };
        id = match i64::from_str(id_str) {
            Ok(id) => id,
            Err(_) => return output_error_msg(res, StatusCode::NotFound, "この部屋は存在しません。"),
        };
    }

    let room = match get_room(id) {
        Ok(room) =>
            match room {
                Some(room) => {
                    room
                },
                None => return output_error_msg(res, StatusCode::NotFound, "この部屋は存在しません。"),
            },
        Err(e) => return output_error(res, e.description()),
    };

    let mut data = vec![];
    {
        match req.origin.read_to_end(&mut data) {
            Ok(_) => (),
            Err(e) => return output_error(res, e.description()),
        }
    }
    let body = String::from_utf8(data).unwrap();
    let posted_stroke: Stroke = match serde_json::from_str(body.as_ref()) {
        Ok(stroke) => stroke,
        Err(e) => return output_error(res, e.description()),
    };
    if posted_stroke.width == 0 || posted_stroke.points.len() == 0 {
        return output_error_msg(res, StatusCode::BadRequest, "リクエストが正しくありません。");
    }

    let strokes = match get_strokes(room.id, 0) {
        Ok(strokes) => strokes,
        Err(e) => return output_error(res, e.description()),
    };

    if strokes.len() == 0 {
        let query = "SELECT COUNT(*) AS cnt FROM `room_owners` WHERE `room_id` = :room_id AND `token_id` = :token_id".to_string();
        let cnt = match DBX.first_exec(query.clone(),
                             params!{"room_id" => room.id,
                                     "token_id" => token.id,
                             }
        ) {
            Ok(row) => {
                match row {
                    Some(mut row) => {
                        row.take("cnt").unwrap()
                    },
                    None => 0,
                }
            },
            Err(e) => return output_error(res, e.description()),
        };
        if cnt == 0 {
            return output_error_msg(res, StatusCode::BadRequest, "他人の作成した部屋に1画目を描くことはできません")
        }
    }
    let mut tx = match DBX.start_transaction(
        true,
        Some(IsolationLevel::ReadCommitted),
        Some(false)
    ) {
        Ok(tx) => tx,
        Err(e) => return output_error(res, e.description()),
    };

    let mut query = "INSERT INTO `strokes` (`room_id`, `width`, `red`, `green`, `blue`, `alpha`)".to_string();
    query += " VALUES(:room_id, :width, :red, :green, :blue, :alpha)";

    let stroke_id = match tx.prep_exec(query.clone(),
                                       params!{"room_id" => room.id,
                                               "width" => posted_stroke.width,
                                               "red" => posted_stroke.red,
                                               "green" => posted_stroke.green,
                                               "blue" => posted_stroke.blue,
                                               "alpha" => posted_stroke.alpha,
                                       }) {
        Ok(row) => row.last_insert_id(),
        Err(e) => return output_error(res, e.description()),
    };

    query = "INSERT INTO `points` (`stroke_id`, `x`, `y`) VALUES (:stroke_id, :x, :y)".to_string();
    for p in posted_stroke.points {
        match tx.prep_exec(query.clone(),
                           params!{"stroke_id" => stroke_id,
                                   "x" => p.x,
                                   "y" => p.y,
                           }) {
            Ok(_) => (),
            Err(e) => return output_error(res, e.description()),
        }
    }

    match tx.commit() {
        Ok(_) => (),
        Err(e) => {
            // FixMe: implement rollback
            // tx.rollback();
            return output_error(res, e.description());
        },
    }

    query = "SELECT `id`, `room_id`, `width`, `red`, `green`, `blue`, `alpha`, `created_at` FROM `strokes`".to_string();
    query += " WHERE `id` = :id";
    let mut s = match DBX.first_exec(query.clone(),
                                 params!{ "id" => stroke_id },
    ) {
        Ok(option) => {
            match option {
                Some(mut row) => {
                    let created_at: NaiveDateTime = row.take("created_at").unwrap();
                    let created_at: DateTime<chrono::Local> = DateTime::from_utc(created_at,
                                                                                 *chrono::Local::now().offset());
                    let created_at = created_at.sub(chrono::Local::now().offset().local_minus_utc());
                    let created_at = format!("{}", created_at.format("%Y-%m-%dT%H:%M:%S%.6f%:z"));
                    // FixMe: Use mysql::from_row()
                    Stroke {
                        id: row.take("id").unwrap(),
                        room_id: row.take("room_id").unwrap(),
                        width: row.take("width").unwrap(),
                        red: row.take("red").unwrap(),
                        green: row.take("green").unwrap(),
                        blue: row.take("blue").unwrap(),
                        alpha: row.take("alpha").unwrap(),
                        created_at: created_at,
                        points: vec!{},
                    }
                },
                None => return output_error(res, "row is none"),
            }
        }
        Err(e) => return output_error(res, e.description()),
    };

    s.points = match get_stroke_points(stroke_id as i64) {
        Ok(points) => points,
        Err(e) => return output_error(res, e.description()),
    };

    #[derive(Debug, Serialize)]
    #[serde(rename="stroke")]
    struct StrokeResp {
        stroke: Stroke,
    };

    let stroke = StrokeResp{ stroke: s };
    let b = match serde_json::to_string(&stroke) {
        Ok(b) => b,
        Err(e) => return output_error(res, e.description()),
    };

    res.set(ContentType(Mime(TopLevel::Application,
                             SubLevel::Json,
                             vec![(Attr::Charset, Value::Utf8)])));
    res.set(StatusCode::Ok);
    res.send(b)
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
    server.options = server.options.output_on_listen(true);
    server.options = server.options.thread_count(Some(16 as usize));
    server.keep_alive_timeout(Some(time::Duration::from_secs(120)));
    let mut router = Nickel::router();


    router.post("/api/csrf_token", post_api_csrf_token);
    router.get("/api/rooms", get_api_rooms);
    router.post("/api/rooms", post_api_rooms);
    router.get("/api/rooms/:id", get_api_rooms_id);
    router.get("/api/stream/rooms/:id", get_api_stream_rooms_id);
    router.post("/api/strokes/rooms/:id", post_api_strokes_rooms_id);

    server.utilize(router);

    let custom_handler: fn(&mut NickelError<()>, &mut Request<()>) -> Action = custom_handler;
    server.handle_error(custom_handler);

    match server.listen("0.0.0.0:8080") {
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
