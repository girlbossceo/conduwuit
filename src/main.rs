#![warn(
    rust_2018_idioms,
    unused_qualifications,
    clippy::cloned_instead_of_copied,
    clippy::str_to_string
)]
#![allow(clippy::suspicious_else_formatting)]
#![deny(clippy::dbg_macro)]

use std::sync::Arc;

use maplit::hashset;
use opentelemetry::trace::{FutureExt, Tracer};
use rocket::{
    catch, catchers,
    figment::{
        providers::{Env, Format, Toml},
        Figment,
    },
    routes, Request,
};
use ruma::api::client::error::ErrorKind;
use tokio::sync::RwLock;
use tracing_subscriber::{prelude::*, EnvFilter};

pub use conduit::*; // Re-export everything from the library crate
pub use rocket::State;

#[cfg(all(not(target_env = "msvc"), feature = "jemalloc"))]
use tikv_jemallocator::Jemalloc;

#[cfg(all(not(target_env = "msvc"), feature = "jemalloc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

fn setup_rocket(config: Figment, data: Arc<RwLock<Database>>) -> rocket::Rocket<rocket::Build> {
    rocket::custom(config)
        .manage(data)
        .mount(
            "/",
            routes![
                client_server::get_supported_versions_route,
                client_server::get_register_available_route,
                client_server::register_route,
                client_server::get_login_types_route,
                client_server::login_route,
                client_server::whoami_route,
                client_server::logout_route,
                client_server::logout_all_route,
                client_server::change_password_route,
                client_server::deactivate_route,
                client_server::third_party_route,
                client_server::get_capabilities_route,
                client_server::get_pushrules_all_route,
                client_server::set_pushrule_route,
                client_server::get_pushrule_route,
                client_server::set_pushrule_enabled_route,
                client_server::get_pushrule_enabled_route,
                client_server::get_pushrule_actions_route,
                client_server::set_pushrule_actions_route,
                client_server::delete_pushrule_route,
                client_server::get_room_event_route,
                client_server::get_room_aliases_route,
                client_server::get_filter_route,
                client_server::create_filter_route,
                client_server::set_global_account_data_route,
                client_server::set_room_account_data_route,
                client_server::get_global_account_data_route,
                client_server::get_room_account_data_route,
                client_server::set_displayname_route,
                client_server::get_displayname_route,
                client_server::set_avatar_url_route,
                client_server::get_avatar_url_route,
                client_server::get_profile_route,
                client_server::set_presence_route,
                client_server::get_presence_route,
                client_server::upload_keys_route,
                client_server::get_keys_route,
                client_server::claim_keys_route,
                client_server::create_backup_route,
                client_server::update_backup_route,
                client_server::delete_backup_route,
                client_server::get_latest_backup_route,
                client_server::get_backup_route,
                client_server::add_backup_key_sessions_route,
                client_server::add_backup_keys_route,
                client_server::delete_backup_key_session_route,
                client_server::delete_backup_key_sessions_route,
                client_server::delete_backup_keys_route,
                client_server::get_backup_key_session_route,
                client_server::get_backup_key_sessions_route,
                client_server::get_backup_keys_route,
                client_server::set_read_marker_route,
                client_server::create_receipt_route,
                client_server::create_typing_event_route,
                client_server::create_room_route,
                client_server::redact_event_route,
                client_server::report_event_route,
                client_server::create_alias_route,
                client_server::delete_alias_route,
                client_server::get_alias_route,
                client_server::join_room_by_id_route,
                client_server::join_room_by_id_or_alias_route,
                client_server::joined_members_route,
                client_server::leave_room_route,
                client_server::forget_room_route,
                client_server::joined_rooms_route,
                client_server::kick_user_route,
                client_server::ban_user_route,
                client_server::unban_user_route,
                client_server::invite_user_route,
                client_server::set_room_visibility_route,
                client_server::get_room_visibility_route,
                client_server::get_public_rooms_route,
                client_server::get_public_rooms_filtered_route,
                client_server::search_users_route,
                client_server::get_member_events_route,
                client_server::get_protocols_route,
                client_server::send_message_event_route,
                client_server::send_state_event_for_key_route,
                client_server::send_state_event_for_empty_key_route,
                client_server::get_state_events_route,
                client_server::get_state_events_for_key_route,
                client_server::get_state_events_for_empty_key_route,
                client_server::sync_events_route,
                client_server::get_context_route,
                client_server::get_message_events_route,
                client_server::search_events_route,
                client_server::turn_server_route,
                client_server::send_event_to_device_route,
                client_server::get_media_config_route,
                client_server::create_content_route,
                client_server::get_content_as_filename_route,
                client_server::get_content_route,
                client_server::get_content_thumbnail_route,
                client_server::get_devices_route,
                client_server::get_device_route,
                client_server::update_device_route,
                client_server::delete_device_route,
                client_server::delete_devices_route,
                client_server::get_tags_route,
                client_server::update_tag_route,
                client_server::delete_tag_route,
                client_server::options_route,
                client_server::upload_signing_keys_route,
                client_server::upload_signatures_route,
                client_server::get_key_changes_route,
                client_server::get_pushers_route,
                client_server::set_pushers_route,
                // client_server::third_party_route,
                client_server::upgrade_room_route,
                server_server::get_server_version_route,
                server_server::get_server_keys_route,
                server_server::get_server_keys_deprecated_route,
                server_server::get_public_rooms_route,
                server_server::get_public_rooms_filtered_route,
                server_server::send_transaction_message_route,
                server_server::get_event_route,
                server_server::get_missing_events_route,
                server_server::get_event_authorization_route,
                server_server::get_room_state_route,
                server_server::get_room_state_ids_route,
                server_server::create_join_event_template_route,
                server_server::create_join_event_v1_route,
                server_server::create_join_event_v2_route,
                server_server::create_invite_route,
                server_server::get_devices_route,
                server_server::get_room_information_route,
                server_server::get_profile_information_route,
                server_server::get_keys_route,
                server_server::claim_keys_route,
            ],
        )
        .register(
            "/",
            catchers![
                not_found_catcher,
                forbidden_catcher,
                unknown_token_catcher,
                missing_token_catcher,
                bad_json_catcher
            ],
        )
}

#[rocket::main]
async fn main() {
    let raw_config =
        Figment::from(default_config())
            .merge(
                Toml::file(Env::var("CONDUIT_CONFIG").expect(
                    "The CONDUIT_CONFIG env var needs to be set. Example: /etc/conduit.toml",
                ))
                .nested(),
            )
            .merge(Env::prefixed("CONDUIT_").global());

    let config = match raw_config.extract::<Config>() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("It looks like your config is invalid. The following error occured while parsing it: {}", e);
            std::process::exit(1);
        }
    };

    let start = async {
        config.warn_deprecated();

        let db = match Database::load_or_create(&config).await {
            Ok(db) => db,
            Err(e) => {
                eprintln!(
                    "The database couldn't be loaded or created. The following error occured: {}",
                    e
                );
                std::process::exit(1);
            }
        };

        let rocket = setup_rocket(raw_config, Arc::clone(&db))
            .ignite()
            .await
            .unwrap();

        Database::start_on_shutdown_tasks(db, rocket.shutdown()).await;

        rocket.launch().await.unwrap();
    };

    if config.allow_jaeger {
        opentelemetry::global::set_text_map_propagator(opentelemetry_jaeger::Propagator::new());
        let tracer = opentelemetry_jaeger::new_pipeline()
            .install_batch(opentelemetry::runtime::Tokio)
            .unwrap();

        let span = tracer.start("conduit");
        start.with_current_context().await;
        drop(span);

        println!("exporting");
        opentelemetry::global::shutdown_tracer_provider();
    } else {
        let registry = tracing_subscriber::Registry::default();
        if config.tracing_flame {
            let (flame_layer, _guard) =
                tracing_flame::FlameLayer::with_file("./tracing.folded").unwrap();
            let flame_layer = flame_layer.with_empty_samples(false);

            let filter_layer = EnvFilter::new("trace,h2=off");

            let subscriber = registry.with(filter_layer).with(flame_layer);
            tracing::subscriber::set_global_default(subscriber).unwrap();
            start.await;
        } else {
            let fmt_layer = tracing_subscriber::fmt::Layer::new();
            let filter_layer = EnvFilter::try_new(&config.log)
                .or_else(|_| EnvFilter::try_new("info"))
                .unwrap();

            let subscriber = registry.with(filter_layer).with(fmt_layer);
            tracing::subscriber::set_global_default(subscriber).unwrap();
            start.await;
        }
    }
}

#[catch(404)]
fn not_found_catcher(_: &Request<'_>) -> String {
    "404 Not Found".to_owned()
}

#[catch(580)]
fn forbidden_catcher() -> Result<()> {
    Err(Error::BadRequest(ErrorKind::Forbidden, "Forbidden."))
}

#[catch(581)]
fn unknown_token_catcher() -> Result<()> {
    Err(Error::BadRequest(
        ErrorKind::UnknownToken { soft_logout: false },
        "Unknown token.",
    ))
}

#[catch(582)]
fn missing_token_catcher() -> Result<()> {
    Err(Error::BadRequest(ErrorKind::MissingToken, "Missing token."))
}

#[catch(583)]
fn bad_json_catcher() -> Result<()> {
    Err(Error::BadRequest(ErrorKind::BadJson, "Bad json."))
}

fn default_config() -> rocket::Config {
    use rocket::config::{LogLevel, Shutdown, Sig};

    rocket::Config {
        // Disable rocket's logging to get only tracing-subscriber's log output
        log_level: LogLevel::Off,
        shutdown: Shutdown {
            // Once shutdown is triggered, this is the amount of seconds before rocket
            // will forcefully start shutting down connections, this gives enough time to /sync
            // requests and the like (which havent gotten the memo, somehow) to still complete gracefully.
            grace: 35,

            // After the grace period, rocket starts shutting down connections, and waits at least this
            // many seconds before forcefully shutting all of them down.
            mercy: 10,

            #[cfg(unix)]
            signals: hashset![Sig::Term, Sig::Int],

            ..Shutdown::default()
        },
        ..rocket::Config::release_default()
    }
}
