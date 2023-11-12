use actix_web::{
    dev::PeerAddr, error, middleware, web, App, Error, HttpRequest, HttpResponse, HttpServer,
};
use awc::Client;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::hash::Hasher;
use std::net::ToSocketAddrs;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use url::Url;

/// Router / load balancer
/// Built off of https://github.com/actix/examples/tree/master/http-proxy
/// Example code was pruned and then additional functionality added for
/// load balancing, static/dynamic routing, etc.

enum RoutingStrategy {
    Hash,
    MovingAverage,
}

const AVERAGE_WINDOW: Duration = Duration::from_secs(10);

struct ResponseTime {
    start: Instant,
    wait: Duration,
}

fn pick_host_from_hash<T: Hash, V>(url: &Vec<V>, key: T) -> usize {
    let mut s = DefaultHasher::new();
    key.hash(&mut s);
    (s.finish() % (url.len() as u64)) as usize
}

fn pick_host_lowest_average(averages: &Vec<Mutex<Duration>>) -> usize {
    let mut lowest = Duration::MAX;
    let mut index = usize::MAX;
    for i in 0..averages.len() {
        {
            // Acquire lock
            let average = averages.get(i).unwrap().lock().unwrap();
            if *average < lowest {
                lowest = *average;
                index = i;
            }
            // Implicit drop
        }
    }
    if index == usize::MAX {
        let mut s = DefaultHasher::new();
        Instant::now().hash(&mut s);
        return (s.finish() % (averages.len() as u64)) as usize;
    }
    index
}

/// Forwards the incoming HTTP request using `awc` (actix web client).
async fn forward(
    req: HttpRequest,
    payload: web::Payload,
    peer_addr: Option<PeerAddr>,
    client: web::Data<Client>,
    hosts: web::Data<Vec<Url>>,
    pool_count: web::Data<Mutex<isize>>,
    times: web::Data<Vec<Mutex<VecDeque<ResponseTime>>>>,
    averages: web::Data<Vec<Mutex<Duration>>>,
    agent_requests: web::Data<HashMap<String, Mutex<VecDeque<Instant>>>>,
) -> Result<HttpResponse, Error> {
    // TODO: parse the agent from the request body/headers
    let agent = "Test123";

    // Calculate traffic share:
    // Acquire lock
    let agent_times = (**agent_requests).get(agent).unwrap().lock().unwrap();
    let agent_total = agent_times.len();
    drop(agent_times);
    // Acquire lock
    let pool = (**pool_count).lock().unwrap();
    let traffic_total = *pool;
    drop(pool);
    let threshhold = (traffic_total as f32) / ((**hosts).len() as f32);

    let routing_strategy = match agent_total as f32 > threshhold {
        true => RoutingStrategy::Hash,
        false => RoutingStrategy::MovingAverage,
    };

    let target_index = match routing_strategy {
        RoutingStrategy::Hash => pick_host_from_hash(&(**hosts), agent),
        RoutingStrategy::MovingAverage => pick_host_lowest_average(&(**averages)),
    };

    let mut target_url = (**hosts).get(target_index).unwrap().clone();

    target_url.set_path(req.uri().path());
    target_url.set_query(req.uri().query());

    let forwarded_req = client
        .request_from(target_url.as_str(), req.head())
        .no_decompress();

    // Probably unnecessary addition: just extra logging info for the coffee app hosts
    let forwarded_req = match peer_addr {
        Some(PeerAddr(addr)) => {
            forwarded_req.append_header(("forwarded", format!("for={}", addr.ip().to_string())))
        }
        None => forwarded_req,
    };

    let request_start = Instant::now();

    let res = forwarded_req
        .send_stream(payload)
        .await
        .map_err(error::ErrorInternalServerError)?;

    let request_time = ResponseTime {
        start: request_start,
        wait: request_start.elapsed(),
    };

    let mut net: isize = 1; // Start at 1 for first add
    let mut new_average: Duration = Duration::ZERO;
    {
        // Acquire lock
        let mut host_times = (**times).get(target_index).unwrap().lock().unwrap();
        // Add new time
        host_times.push_back(request_time);
        // Drop old times
        for i in 0..host_times.len() {
            if host_times
                .get(i)
                .unwrap()
                .start
                .duration_since(request_start)
                < AVERAGE_WINDOW
            {
                break;
            }
            host_times.pop_front();
            net -= 1;
        }
        for i in 0..host_times.len() {
            new_average += host_times.get(i).unwrap().wait;
        }
        new_average /= host_times.len() as u32;
        // Implicit mutex drop
    }
    {
        // Acquire lock
        let mut average = (**averages).get(target_index).unwrap().lock().unwrap();
        *average = new_average;
        // Implicit mutex drop
    }
    {
        // Acquire lock
        let mut count = (**pool_count).lock().unwrap();
        *count += net;
        // Implicit mutex drop
    }
    {
        // Acquire lock
        let mut agent_times = (**agent_requests).get(agent).unwrap().lock().unwrap();
        // Add new time
        agent_times.push_back(request_start);
        // Drop old times
        for i in 0..agent_times.len() {
            if agent_times.get(i).unwrap().duration_since(request_start) < AVERAGE_WINDOW {
                break;
            }
            agent_times.pop_front();
        }
        // Implicit mutex drop
    }

    let mut client_resp = HttpResponse::build(res.status());
    // Remove `Connection` as per
    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Connection#Directives
    for (header_name, header_value) in res.headers().iter().filter(|(h, _)| *h != "connection") {
        client_resp.insert_header((header_name.clone(), header_value.clone()));
    }

    Ok(client_resp.streaming(res))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let forward_socket_addr = ("127.0.0.1", 3000)
        .to_socket_addrs()?
        .next()
        .expect("given forwarding address was not valid");
    // TODO: support https
    let forward_url = format!("http://{forward_socket_addr}");
    let forward_url = Url::parse(&forward_url).unwrap();
    HttpServer::new(move || {
        let hosts = vec![
            forward_url.clone(),
            forward_url.clone(),
            forward_url.clone(),
            forward_url.clone(),
        ];
        let pool_count = Mutex::new(0 as isize);
        let mut times: Vec<Mutex<VecDeque<ResponseTime>>> = Vec::new();
        let mut averages: Vec<Mutex<Duration>> = Vec::new();
        for _ in 0..hosts.len() {
            times.push(Mutex::new(VecDeque::new()));
            averages.push(Mutex::new(Duration::ZERO));
        }
        let agent_requests: HashMap<String, Mutex<VecDeque<Instant>>> = HashMap::new();
        App::new()
            .app_data(web::Data::new(Client::default()))
            .app_data(web::Data::new(hosts))
            .app_data(web::Data::new(pool_count))
            .app_data(web::Data::new(times))
            .app_data(web::Data::new(averages))
            .app_data(web::Data::new(agent_requests))
            .wrap(middleware::Logger::default())
            .default_service(web::to(forward))
    })
    .bind(("127.0.0.1", 8080))?
    .workers(2)
    .run()
    .await
}
