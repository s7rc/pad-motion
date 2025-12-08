use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Instant, Duration};
use std::thread;
use std::fs;
use std::collections::HashMap;

use gilrs::{Gilrs, Button, Axis};
use multiinput::{RawInputManager, RawEvent};

use pad_motion::protocol::*;
use pad_motion::server::*;

// Default Configuration
struct AppConfig {
    sensitivity: f32,
    invert_x: f32,      // 1.0 or -1.0
    invert_y: f32,      // 1.0 or -1.0
    gravity_axis: u8,   // 0=X, 1=Y, 2=Z
    gravity_amount: f32 // Usually 9.81
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            sensitivity: 5.0,
            invert_x: -1.0, // Flipped based on your feedback
            invert_y: 1.0,  // Flipped based on your feedback
            gravity_axis: 1, // 1 = Y-Axis (Upright/Remote style) to fix "X" movement
            gravity_amount: 9.81,
        }
    }
}

fn main() {
  let running = Arc::new(AtomicBool::new(true));

  {
    let running = running.clone();
    ctrlc::set_handler(move || {
      running.store(false, Ordering::SeqCst);
    }).expect("Error setting Ctrl-C handler");
  }

  let server = Arc::new(Server::new(None, None).unwrap());
  let server_thread_join_handle = {
    let server = server.clone();
    server.start(running.clone())
  };

  let controller_info = ControllerInfo {
    slot_state: SlotState::Connected,
    device_type: DeviceType::FullGyro,
    connection_type: ConnectionType::USB,
    .. Default::default()
  };
  server.update_controller_info(controller_info);

  fn to_stick_value(input: f32) -> u8 {
    (input * 127.0 + 127.0) as u8 
  }

  // Shared Config (Thread Safe)
  let config = Arc::new(Mutex::new(AppConfig::default()));

  // --- CONFIG FILE WATCHER ---
  // Reads 'config.txt' every second.
  // Format: key=value (e.g., sensitivity=5.0)
  {
      let config = config.clone();
      let running = running.clone();
      thread::spawn(move || {
          while running.load(Ordering::Relaxed) {
              thread::sleep(Duration::from_secs(1));
              
              if let Ok(contents) = fs::read_to_string("config.txt") {
                  let mut new_config = AppConfig::default(); // Reset to defaults first
                  
                  for line in contents.lines() {
                      if let Some((key, value)) = line.split_once('=') {
                          let key = key.trim();
                          let val = value.trim().parse::<f32>().unwrap_or(0.0);
                          
                          match key {
                              "sensitivity" => new_config.sensitivity = val,
                              "invert_x" => new_config.invert_x = if val > 0.0 { 1.0 } else { -1.0 },
                              "invert_y" => new_config.invert_y = if val > 0.0 { 1.0 } else { -1.0 },
                              "gravity_axis" => new_config.gravity_axis = val as u8,
                              "gravity_amount" => new_config.gravity_amount = val,
                              _ => {}
                          }
                      }
                  }
                  
                  // Update the shared config
                  if let Ok(mut c) = config.lock() {
                      *c = new_config;
                  }
              }
          }
      });
  }

  let mut gilrs = Gilrs::new().unwrap();
  let mut mouse_manager = RawInputManager::new().unwrap();
  mouse_manager.register_devices(multiinput::DeviceType::Mice);

  let now = Instant::now();
  while running.load(Ordering::SeqCst) {
    // Consume controller events
    while let Some(_event) = gilrs.next_event() {
    }

    let mut delta_rotation_x = 0.0;
    let mut delta_rotation_y = 0.0;
    
    while let Some(event) = mouse_manager.get_event() {
      match event {
        RawEvent::MouseMoveEvent(_mouse_id, delta_x, delta_y) => {
          delta_rotation_x += delta_x as f32;
          delta_rotation_y += delta_y as f32;
        },
        _ => ()
      }
    }

    // Capture current config snapshot
    let (sens, inv_x, inv_y, g_axis, g_val) = {
        let c = config.lock().unwrap();
        (c.sensitivity, c.invert_x, c.invert_y, c.gravity_axis, c.gravity_amount)
    };

    // Apply Sensitivity & Inversion
    let gyro_yaw = delta_rotation_x * sens * inv_x;
    let gyro_pitch = delta_rotation_y * sens * inv_y;

    // Apply Gravity Vector (Fixes the "X vs +" rotation issue)
    let (accel_x, accel_y, accel_z) = match g_axis {
        0 => (g_val, 0.0, 0.0), // X-Axis (Sideways)
        1 => (0.0, g_val, 0.0), // Y-Axis (Upright/Pointer) <- DEFAULT
        _ => (0.0, 0.0, g_val), // Z-Axis (Flat)
    };

    let first_gamepad = gilrs.gamepads().next();
    let controller_data = {
      if let Some((_id, gamepad)) = first_gamepad {
        let analog_button_value = |button| {
          gamepad.button_data(button).map(|data| (data.value() * 255.0) as u8).unwrap_or(0)
        };

        ControllerData {
          connected: true,
          d_pad_left: gamepad.is_pressed(Button::DPadLeft),
          d_pad_down: gamepad.is_pressed(Button::DPadDown),
          d_pad_right: gamepad.is_pressed(Button::DPadRight),
          d_pad_up: gamepad.is_pressed(Button::DPadUp),
          start: gamepad.is_pressed(Button::Start),
          right_stick_button: gamepad.is_pressed(Button::RightThumb),
          left_stick_button: gamepad.is_pressed(Button::LeftThumb),
          select:  gamepad.is_pressed(Button::Select),
          triangle: gamepad.is_pressed(Button::North),
          circle: gamepad.is_pressed(Button::East),
          cross: gamepad.is_pressed(Button::South),
          square: gamepad.is_pressed(Button::West),
          r1: gamepad.is_pressed(Button::RightTrigger),
          l1: gamepad.is_pressed(Button::LeftTrigger),
          r2: gamepad.is_pressed(Button::RightTrigger2),
          l2: gamepad.is_pressed(Button::LeftTrigger2),
          ps: analog_button_value(Button::Mode),
          left_stick_x: to_stick_value(gamepad.value(Axis::LeftStickX)),
          left_stick_y: to_stick_value(gamepad.value(Axis::LeftStickY)),
          right_stick_x: to_stick_value(gamepad.value(Axis::RightStickX)),
          right_stick_y: to_stick_value(gamepad.value(Axis::RightStickY)),
          analog_d_pad_left: analog_button_value(Button::DPadLeft),
          analog_d_pad_down: analog_button_value(Button::DPadDown),
          analog_d_pad_right: analog_button_value(Button::DPadRight),
          analog_d_pad_up: analog_button_value(Button::DPadUp),
          analog_triangle: analog_button_value(Button::North),
          analog_circle: analog_button_value(Button::East),
          analog_cross: analog_button_value(Button::South),
          analog_square: analog_button_value(Button::West),
          analog_r1: analog_button_value(Button::RightTrigger),
          analog_l1: analog_button_value(Button::LeftTrigger),
          analog_r2: analog_button_value(Button::RightTrigger2),
          analog_l2: analog_button_value(Button::LeftTrigger2),
          motion_data_timestamp: now.elapsed().as_micros() as u64,
          
          accelerometer_x: accel_x,
          accelerometer_y: accel_y,
          accelerometer_z: accel_z,
          
          gyroscope_pitch: gyro_pitch,
          gyroscope_yaw: gyro_yaw,
          gyroscope_roll: 0.0,

          .. Default::default()
        }
      } else {
        ControllerData {
          connected: true,
          motion_data_timestamp: now.elapsed().as_micros() as u64,
          
          accelerometer_x: accel_x,
          accelerometer_y: accel_y,
          accelerometer_z: accel_z,

          gyroscope_pitch: gyro_pitch,
          gyroscope_yaw: gyro_yaw,
          gyroscope_roll: 0.0,

          .. Default::default()
        }
      }
    };

    server.update_controller_data(0, controller_data);
    std::thread::sleep(Duration::from_millis(1));
  }

  server_thread_join_handle.join().unwrap();
}
