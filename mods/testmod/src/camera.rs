use weft_sdk::*;

use std::sync::{Arc, Mutex};

static ZOOM_KEY: Key = Key::named("zoom");
static ZOOM_IN: Key = Key::named("zoom_in");
static ZOOM_OUT: Key = Key::named("zoom_out");

const ZOOM_PER_NOTCH: f64 = 0.1;
const ZOOM_RESPONSE: f32 = 14.0;
const DEFAULT_ZOOM: f64 = 5.0;

pub fn register(r: &mut Registry) {
    r.key(&ZOOM_KEY, "Z").tag(abi::comp::key::DEBUG);

    r.key(&ZOOM_IN, "KeyZ+wheel_up")
        // .with(&ZOOM_KEY)
        .tag(abi::comp::key::DEBUG);

    r.key(&ZOOM_OUT, "KeyZ+wheel_down")
        // .with(&ZOOM_KEY)
        .tag(abi::comp::key::DEBUG);

    struct ZoomState {
        amount: f64,
        active: bool,
    }

    let zoom = Arc::new(Mutex::new(ZoomState { amount: 0.0, active: false }));

    let frame_zoom = Arc::clone(&zoom);

    r.client_frame(move |c| {
        let mut z = frame_zoom.lock().unwrap();
        let held = c.key_held(&ZOOM_KEY);

        if held {
            // Initialise zoom when first pressed
            if !z.active {
                z.amount = DEFAULT_ZOOM.ln();
            }

            z.amount += f64::from(c.scroll()) * ZOOM_PER_NOTCH;
        } else {
            z.amount = 0.0;
        }

        z.active = held;
    });

    let shot_zoom = Arc::clone(&zoom);

    use abi::comp::camera::*;

    r.edit(&cameras::FIRST_PERSON)
        .shot(Shot::new(&[HAND, HUD, EFFECTS], move |a| {
            let target_fov = (-shot_zoom.lock().unwrap().amount).exp() as f32;

            let blend = 1.0 - (-ZOOM_RESPONSE * a.dt.max(0.0)).exp();
            let fov = a.out.fov + (target_fov - a.out.fov) * blend;

            a.out = abi::CameraPose { pos: a.eye, yaw: a.eye_yaw, pitch: a.eye_pitch, fov, ..abi::CameraPose::DEFAULT };
        }));
}
