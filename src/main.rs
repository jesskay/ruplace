extern crate reqwest;
extern crate serde_json;
#[macro_use]
extern crate hyper;
#[macro_use]
extern crate serde_derive;
extern crate png;
extern crate rand;

use std::env;
use std::collections::HashMap;
use std::thread;
use std::time::Duration;
use std::error::Error;
use std::process;

use serde_json::Value;
use reqwest::{RequestBuilder, Client};
use hyper::header::Cookie;
use png::HasParameters;

header! { (XModhash, "x-modhash") => [String] }

#[derive(Debug)]
struct RedditSession {
    pub modhash: String,
    pub cookie: String
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
struct TargetJson {
    pub major_version: u32,
    pub minor_version: u32,
    pub x: u32,
    pub y: u32,
    pub image: String
}

const TARGET_JSON_URL: &'static str = "https://raw.githubusercontent.com/jesskay/ruplace-target/master/ruplace.json";

const PALETTE: [[u8; 4]; 17] = [
    [255, 255, 255, 255],
    [228, 228, 228, 255],
    [136, 136, 136, 255],
    [ 34,  34,  34, 255],
    [255, 167, 209, 255],
    [229,   0,   0, 255],
    [229, 149,   0, 255],
    [160, 106,  66, 255],
    [229, 217,   0, 255],
    [148, 224,  68, 255],
    [  2, 190,   1, 255],
    [  0, 211, 221, 255],
    [  0, 131, 199, 255],
    [  0,   0, 234, 255],
    [207, 110, 228, 255],
    [130,   0, 128, 255],
    [  0,   0,   0,   0],
];

const MAJOR_VERSION: u32 = 1;
const MINOR_VERSION: u32 = 0;

const DEBUG: bool = false;

fn color_to_index(color: &[u8]) -> u8 {
    if color[3] < 128 {
        return 16;
    }
    PALETTE.iter().enumerate().map(|(index, p)| {
        (index, p.iter().zip(color.iter()).map(|(a, b)| {
            let diff = *a as i32 - *b as i32;
            diff*diff
        }).sum::<i32>())
    }).min_by_key(|&(_, diff)| diff).expect("4 components").0 as u8
}

fn main() {
    let mut args = env::args().skip(1);
    let username = args.next().expect("<username> argument");
    let password = args.next().expect("<password> argument");

    let mut target = TargetJson {
        major_version: MAJOR_VERSION,
        minor_version: MINOR_VERSION,
        x: 0,
        y: 0,
        image: String::new()
    };
    let mut width = 0;
    let mut height = 0;
    let mut target_image = Vec::new();

    let mut board = Vec::new();
    board.resize(1000*1000/2, 0u8);

    loop {
        let mut try_place_pixel = || -> Result<(), Box<Error>> {
            let new_target: TargetJson = reqwest::get(TARGET_JSON_URL)?.json()?;
            if new_target.major_version > MAJOR_VERSION {
                println!("New major version is available. Must update!");
                process::exit(1);
            }

            if new_target.minor_version > MINOR_VERSION {
                println!("New minor version is available. Update when convenient.");
            }

            if new_target != target {
                target = new_target;
                let mut decoder = png::Decoder::new(reqwest::get(&target.image)?);
                decoder.set(png::TRANSFORM_EXPAND | png::TRANSFORM_GRAY_TO_RGB | png::TRANSFORM_PACKING | png::TRANSFORM_STRIP_16);
                let (info, mut reader) = decoder.read_info()?;
                width = info.width;
                height = info.height;
                let mut buffer = Vec::new();
                buffer.resize(info.buffer_size(), 0u8);
                reader.next_frame(&mut *buffer)?;

                target_image.truncate(0);
                target_image.reserve_exact((width*height) as usize);

                match info.color_type {
                    png::ColorType::RGB => {
                        for color in buffer.chunks(3) {
                            let c = [color[0], color[1], color[2], 255];
                            target_image.push(color_to_index(&c));
                        }
                    },
                    png::ColorType::RGBA => {
                        for color in buffer.chunks(4) {
                            target_image.push(color_to_index(color));
                        }
                    },
                    _ => return Err("Reference image has unsupported color type".into())
                }
            }

            if DEBUG {
                println!("{:?}", target);
            }

            fetch_board(&mut board)?;
            let (x, y, color) = pick_random_pixel(&board,
                target.x, target.y, width, height, &target_image)?;

            println!("Placing pixel: ({}, {}) - {}", x, y, color);

            let session = reddit_login(&username, &password)?;
            let delay = place_pixel(x, y, color, &session)?;

            println!("Sleeping for {} seconds...", delay);
            thread::sleep(Duration::from_secs(delay as u64));

            Ok(())
        };

        if let Err(e) = try_place_pixel() {
            println!("{} - sleeping for 10 seconds", e);
            thread::sleep(Duration::from_secs(10));
        }
    }
}

fn sample_board(board: &[u8], x: u32, y: u32) -> u8 {
    let v = board[((y as usize))*500 + (x as usize)/2];
    if x % 2 == 0 {
        v >> 4
    } else {
        v & 0xF
    }
}

fn sample_target(target: &[u8], x: u32, y: u32, width: u32) -> u8 {
    target[(y as usize)*(width as usize) + (x as usize)]
}

fn pick_random_pixel(board: &[u8], x: u32, y: u32, width: u32, height: u32, target_image: &[u8])
                     -> Result<(u32, u32, u8), Box<Error>> {
    use rand::Rng;
    let mut count = 0;
    let mut solid = 0;
    let hex = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "a", "b", "c", "d", "e", "f", "."];
    for py in 0..height {
        let mut sb = String::new();
        let mut st = String::new();
        for px in 0..width {
            let bp = sample_board(board, x + px, y + py);
            let tp = sample_target(target_image, px, py, width);
            if DEBUG {
                sb += hex[bp as usize];
                st += hex[tp as usize];
            }
            if tp != 16 && tp != bp {
                count += 1;
            }
            if tp != 16 {
                solid += 1;
            }
        }
        if DEBUG {
            println!("{} - {}", sb, st);
        }
    }
    let done = solid - count;
    let percentage_done = ((done*1000/solid) as f64)*0.1;
    println!("Progress: {}/{} ({:.1}%)", done, solid, percentage_done);

    if count == 0 || DEBUG {
        return Err("Nothing to do (for now)".into());
    }

    let mut index = rand::thread_rng().gen_range(0, count);
    for py in 0..height {
        for px in 0..width {
            let bp = sample_board(board, x + px, y + py);
            let tp = sample_target(target_image, px, py, width);
            if tp != 16 && tp != bp {
                index -= 1;
                if index == 0  {
                    return Ok((px + x, py + y, tp));
                }
            }
        }
    }

    Err("Nothing to do (for now)".into())
}

fn fetch_board(board: &mut Vec<u8>) -> Result<(), Box<Error>> {
    use std::io::Read;
    let mut file = reqwest::get("https://www.reddit.com/api/place/board-bitmap")?;
    file.read_exact(&mut board[0..4])?;
    file.read_exact(&mut *board)?;
    Ok(())
}

fn place_pixel(x: u32, y: u32, color: u8, session: &RedditSession) -> Result<u32, Box<Error>> {
    let client = Client::new()?;

    let mut params = HashMap::new();
    params.insert("x", x);
    params.insert("y", y);
    params.insert("color", color as u32);

    let response: Value = reddit_auth(client.post("https://www.reddit.com/api/place/draw.json"), session)
        .form(&params)
        .send()?
        .json()?;
    Ok(response.get("wait_seconds").and_then(Value::as_u64)
                                   .ok_or("Did not receive wait time")? as u32)
}

fn reddit_login(username: &str, password: &str) -> Result<RedditSession, Box<Error>> {
    let client = Client::new()?;

    let mut params = HashMap::new();
    params.insert("op", "login-main");
    params.insert("user", &username);
    params.insert("passwd", &password);
    params.insert("rem", "on");
    params.insert("api_type", "json");

    let response: Value = client.post(&format!("https://www.reddit.com/api/login/{}", username))
        .form(&params)
        .send()?
        .json()?;

    let inner = response.get("json").ok_or("No json returned from login")?;
    let errors = inner.get("errors").and_then(Value::as_array).ok_or("No errors returned from login")?;
    if errors.len() > 0 {
        return Err(format!("Login errors: {:?}", errors).into());
    }
    let data = inner.get("data").ok_or("No data returned from login")?;

    Ok(RedditSession {
        modhash: data.get("modhash").and_then(Value::as_str)
                                    .ok_or("No modhash returned from login")?.to_owned(),
        cookie: data.get("cookie").and_then(Value::as_str)
                                  .ok_or("No cookie returned from login")?.to_owned(),
    })
}

fn reddit_auth(req: RequestBuilder, session: &RedditSession) -> RequestBuilder {
    req
        .header(XModhash(session.modhash.clone()))
        .header(Cookie(vec![
            format!("reddit_session={}", session.cookie)
        ]))
}
