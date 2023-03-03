// SPDX-FileCopyrightText: © 2023 Alexander König <alex@lisas.de>
// SPDX-License-Identifier: MIT

use std::io::{Error, ErrorKind};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::{fs, path::Path, process, thread, time::Duration};

use paho_mqtt as mqtt;
use rppal::gpio::{Gpio, OutputPin};
use serde_json as json;

const REQUEST_NONE: i32 = -1;
const REQUEST_PLAY: i32 = 0;
const REQUEST_TERMINATE: i32 = 1;
const REQUEST_CANCEL: i32 = 2;

fn mqtt_reconnect(client: &mqtt::Client) -> bool {
    println!("Connection to MQTT broker lost. Reconnecting...");
    loop {
        thread::sleep(Duration::from_millis(3000));
        if client.reconnect().is_ok() {
            println!("Connection to MQTT broker restored.");
            return true;
        }
    }
}

fn set_realtime() {
    let pid = process::id() as i32;
    let result = scheduler::set_policy(pid, scheduler::Policy::Fifo, 99);
    assert!(result.is_ok(), "failed to acquire realtime priority");
}

fn find_config() -> Result<String, Error> {
    let config_file_name = "buzzd.json";

    if Path::new(config_file_name).exists() {
        return Ok(String::from(config_file_name));
    }

    let user_config_dir = dirs::config_dir();

    if let Some(mut user_config_path) = user_config_dir {
        user_config_path.push(config_file_name);

        let user_config_filename = String::from(user_config_path.to_str().unwrap());

        if Path::new(&user_config_filename).exists() {
            return Ok(user_config_filename);
        }
    }

    let mut global_config_path = PathBuf::from("/etc");
    global_config_path.push(config_file_name);

    let global_config_filename = String::from(global_config_path.to_str().unwrap());

    if Path::new(&global_config_filename).exists() {
        return Ok(global_config_filename);
    }

    let msg = "configuration file not found";
    Err(Error::new(ErrorKind::NotFound, msg))
}

fn load_config() -> json::Value {
    let config_file_name = find_config().expect("couldn't find buzzd configuration");

    println!("Using configuration: \'{config_file_name}\'");

    let json = fs::read_to_string(config_file_name).unwrap_or_else(|_| String::from("{}"));
    let config: json::Value = json::from_str(&json).expect("unable to parse configuration file");

    config
}

fn play_pattern(
    rx: &Receiver<PlayRequest>,
    pin: &mut OutputPin,
    config: &json::Value,
    pattern: &str,
    repeat_override: i32,
) -> PlayRequest {
    if let Some(pattern_configs) = config["patterns"].as_array() {
        for pattern_config in pattern_configs {
            if pattern_config["name"]
                .as_str()
                .expect("pattern without name")
                .eq_ignore_ascii_case(pattern)
            {
                if let Some(rhythm) = pattern_config["rhythm"].as_array() {
                    let repetitions = if repeat_override < 0 {
                        pattern_config["repeat"]
                            .as_u64()
                            .expect("invalid repeat value for pattern")
                    } else {
                        (i64::from(repeat_override)).unsigned_abs()
                    };

                    for i in 0..=repetitions {
                        let mut on = true;

                        for step in rhythm {
                            if on {
                                pin.set_low();
                            } else {
                                pin.set_high();
                            }

                            on = !on;

                            thread::sleep(Duration::from_millis(
                                step.as_u64().expect("invalid step in rhythm for pattern"),
                            ));

                            // allow interruption of the pattern through a new request
                            if let Ok(play_request) = rx.try_recv() {
                                pin.set_high();
                                return play_request;
                            }
                        }

                        pin.set_high();

                        if i != repetitions {
                            thread::sleep(Duration::from_millis(
                                config["pause"].as_u64().unwrap_or(500),
                            ));
                        }

                        // allow interruption of the repetitions through a new request
                        if let Ok(play_request) = rx.try_recv() {
                            pin.set_high();
                            return play_request;
                        }
                    }
                }

                break;
            }
        }
    }

    pin.set_high();

    PlayRequest {
        request: REQUEST_NONE,
        pattern: String::from(""),
        repeat_override: -1,
    }
}

fn setup_mqtt_client(config: &json::Value) -> mqtt::Client {
    let mqtt_broker = config["mqtt"]["broker"].as_str().unwrap_or("localhost");
    let mqtt_topic = config["mqtt"]["topic"].as_str().unwrap_or("buzzd");

    let mqtt_create_options = mqtt::CreateOptionsBuilder::new()
        .server_uri(mqtt_broker)
        .client_id("buzzd")
        .persistence(None)
        .finalize();

    let mqtt_client =
        mqtt::Client::new(mqtt_create_options).expect("failed to instantiate MQTT client");
    let mqtt_connect_options = mqtt::ConnectOptionsBuilder::new()
        .keep_alive_interval(Duration::from_millis(30000))
        .clean_session(false)
        .finalize();

    mqtt_client
        .connect(mqtt_connect_options)
        .expect("failed to connect to MQTT broker");
    mqtt_client.subscribe(mqtt_topic, 1).unwrap_or_else(|_| {
        mqtt_client.disconnect(None).unwrap();
        panic!("could not subscribe to MQTT topic");
    });

    mqtt_client
}

fn setup_buzzer_pin(config: &json::Value) -> OutputPin {
    // setup GPIO
    let gpio = Gpio::new().expect("failed to access GPIO");

    let pin_id = config["gpio"]
        .as_u64()
        .expect("configuration does not define buzzer GPIO pin") as u8;

    let mut pin = gpio
        .get(pin_id)
        .expect("failed to access buzzer GPIO pin")
        .into_output();
    // silence the buzzer until a pattern replay is requested
    pin.set_high();

    pin
}

struct PlayRequest {
    request: i32,
    pattern: String,
    repeat_override: i32,
}

fn playback_loop(
    rx: &Receiver<PlayRequest>,
    interrupt_request: &mut PlayRequest,
    pin: &mut OutputPin,
    config: &json::Value,
) -> bool {
    let mut play_request = PlayRequest {
        request: REQUEST_NONE,
        pattern: String::from(""),
        repeat_override: -1,
    };
    if interrupt_request.request != REQUEST_NONE {
        play_request.request = interrupt_request.request;
        play_request.pattern = interrupt_request.pattern.clone();
        play_request.repeat_override = interrupt_request.repeat_override;
    } else {
        let received_request = rx.recv().unwrap();
        play_request.request = received_request.request;
        play_request.pattern = received_request.pattern.clone();
        play_request.repeat_override = received_request.repeat_override;
    };

    if play_request.request == REQUEST_PLAY {
        *interrupt_request = play_pattern(
            rx,
            pin,
            config,
            &play_request.pattern,
            play_request.repeat_override,
        );
        return true;
    }

    // reset interrupt request
    interrupt_request.request = REQUEST_NONE;

    if play_request.request == REQUEST_TERMINATE {
        return false;
    }

    true
}

fn main() {
    // read configuration
    let config = load_config();
    let (tx, rx) = mpsc::channel::<PlayRequest>();

    // create MQTT client and connect to broker
    let mqtt_client = setup_mqtt_client(&config);
    let mqtt_receiver = mqtt_client.start_consuming();

    // handle termination
    let cltrc_handler_client = mqtt_client.clone();
    ctrlc::set_handler(move || {
        cltrc_handler_client.stop_consuming();
    })
    .expect("failed to setup signal handler");

    // acquire GPIO pin
    let mut pin = setup_buzzer_pin(&config);

    let thread_handle = thread::spawn(move || {
        // set realtime scheduling policy to stay in rhythm
        set_realtime();
        let mut running = true;
        let mut interrupt_request = PlayRequest {
            request: REQUEST_NONE,
            pattern: String::from(""),
            repeat_override: -1,
        };

        while running {
            running = playback_loop(&rx, &mut interrupt_request, &mut pin, &config);
        }
    });

    // ready to serve, process MQTT messages
    for pattern_command in mqtt_receiver.iter() {
        if let Some(pattern_command) = pattern_command {
            let pattern_command_string = pattern_command.payload_str();
            let mut pattern_command_iter = pattern_command_string.split_whitespace();
            let pattern = pattern_command_iter.next().unwrap_or("error");
            let repeat_override = pattern_command_iter
                .next()
                .unwrap_or("-1")
                .parse()
                .unwrap_or(-1);

            let mut request = PlayRequest {
                request: REQUEST_PLAY,
                pattern: String::from(pattern),
                repeat_override,
            };

            if pattern.eq("_") {
                request.request = REQUEST_CANCEL;
            }

            tx.send(request).unwrap();
        } else if mqtt_client.is_connected() || !mqtt_reconnect(&mqtt_client) {
            break;
        }
    }

    tx.send(PlayRequest {
        request: REQUEST_TERMINATE,
        pattern: String::from(""),
        repeat_override: -1,
    })
    .unwrap();

    if mqtt_client.is_connected() {
        mqtt_client.disconnect(None).unwrap();
    }

    thread_handle.join().unwrap();
}
