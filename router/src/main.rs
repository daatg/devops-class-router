use std::net::ToSocketAddrs;
use actix_web::{
    dev::PeerAddr, error, middleware, web, App, Error, HttpRequest, HttpResponse,
    HttpServer,
};
use awc::Client;
use url::Url;
use std::hash::Hash;

/// Router / load balancer
/// Built off of https://github.com/actix/examples/tree/master/http-proxy
/// Example code was pruned and then additional functionality added for 
/// load balancing, static/dynamic routing, etc.
fn pick_url_static<T: Hash>(url: Vec<Url>, key: T) -> Url {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;
    let mut s = DefaultHasher::new();
    key.hash(&mut s);
    let pick = s.finish() % (url.len() as u64);
    url.get(pick as usize).unwrap().clone()
}

/// Forwards the incoming HTTP request using `awc` (actix web client).
async fn forward(
    req: HttpRequest,
    payload: web::Payload,
    peer_addr: Option<PeerAddr>,
    url: web::Data<Vec<Url>>,
    client: web::Data<Client>,
) -> Result<HttpResponse, Error> {
    let mut new_url = pick_url_static((**url).clone(), "Test123");
    new_url.set_path(req.uri().path());
    new_url.set_query(req.uri().query());

    let forwarded_req = client
        .request_from(new_url.as_str(), req.head())
        .no_decompress();

    // Probably unnecessary addition: just extra logging info for the coffee app hosts
    let forwarded_req = match peer_addr {
        Some(PeerAddr(addr)) => {
            forwarded_req.append_header(("forwarded", format!("for={}", addr.ip().to_string())))
        }
        None => forwarded_req,
    };

    let res = forwarded_req
        .send_stream(payload)
        .await
        .map_err(error::ErrorInternalServerError)?;

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
    let forward_url = format!("http://{forward_socket_addr}");
    let forward_url = Url::parse(&forward_url).unwrap();

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(Client::default()))
            .app_data(web::Data::new(
                vec![
                    forward_url.clone(),
                    forward_url.clone(),
                    forward_url.clone(),
                    forward_url.clone()
                ]
            ))
            .wrap(middleware::Logger::default())
            .default_service(web::to(forward))
    })
    .bind(("127.0.0.1", 8080))?
    .workers(2)
    .run()
    .await
}