use super::events::Event;
use super::events::VectorDirection;
use app::constants::DEAD_ZONE;
use app::constants::*;
use core::clock::Seconds;
use core::geometry::*;
use core::view::ViewTransform;
use core::view::WorldTransform;
use frontend::input;

use super::events::Event::*;
use frontend::input::Key::*;

const KEY_HELD_MAP: &[(input::Key, Event)] = &[(W, CamUp(1.)), (S, CamDown(1.)), (A, CamLeft(1.)), (D, CamRight(1.))];

const KEY_PRESSED_ONCE_MAP: &[(input::Key, Event)] = &[
	(F5, Reload),
	(F1, ToggleGui),
	(GamepadL3, ToggleGui),
	(N0, CamReset),
	(Home, CamReset),
	(GamepadSelect, ToggleCapture),
	(GamepadStart, TogglePause),
	(KpHome, CamReset),
	(GamepadDPadUp, ZoomIn),
	(GamepadDPadDown, ZoomOut),
	(GamepadR3, ZoomReset),
	(Plus, ZoomIn),
	(Minus, ZoomOut),
	(N1, ZoomReset),
	(F6, SaveGenePoolToFile),
	(F7, SaveWorldToFile),
	(F8, RestartFromCheckpoint),
	(F10, ToggleDebug),
	(F12, ToggleCapture),
	(GamepadStart, ToggleDebug),
	(Z, DeselectAll),
	(L, NextLight),
	(B, NextBackground),
	(K, PrevLight),
	(V, PrevBackground),
	(G, PrevSpeedFactor),
	(GamepadL1, PrevSpeedFactor),
	(H, NextSpeedFactor),
	(GamepadR1, NextSpeedFactor),
	(P, TogglePause),
	(Esc, AppQuit),
	(MouseScrollUp, ZoomIn),
	(MouseScrollDown, ZoomOut),
];

pub struct DefaultController {}

pub trait InputController {
	fn update<V, W, I>(input_state: &I, view_transform: &V, world_transform: &W, dt: Seconds) -> Vec<Event>
	where
		V: ViewTransform,
		W: WorldTransform,
		I: input::InputRead;
}

impl DefaultController {
	fn events_on_key_held<I>(input_state: &I, events: &mut Vec<Event>)
	where I: input::InputRead {
		for (key_held, event) in KEY_HELD_MAP {
			if input_state.key_pressed(*key_held) {
				events.push(*event);
			}
		}
	}

	fn events_on_key_pressed_once<I>(input_state: &I, events: &mut Vec<Event>)
	where I: input::InputRead {
		for (key_pressed, event) in KEY_PRESSED_ONCE_MAP {
			if input_state.key_pressed(*key_pressed) {
				events.push(*event);
			}
		}
	}

	fn events_on_mouse_move<V, W, I>(
		input_state: &I,
		events: &mut Vec<Event>,
		view_transform: &V,
		world_transform: &W,
		dt: Seconds,
	) -> (bool, Position)
	where
		V: ViewTransform,
		W: WorldTransform,
		I: input::InputRead,
	{
		let mouse_window_pos = input_state.mouse_position();
		let mouse_view_pos = view_transform.to_view(mouse_window_pos);
		let mouse_world_pos = world_transform.to_world(mouse_view_pos);

		let mouse_left_pressed = input_state.key_pressed(input::Key::MouseLeft) && !input_state.any_ctrl_pressed();
		if input_state.key_once(input::Key::MouseLeft) && input_state.any_ctrl_pressed() {
			events.push(Event::PickMinion(mouse_world_pos));
		};

		if input_state.key_once(input::Key::MouseMiddle) {
			if input_state.any_ctrl_pressed() {
				events.push(Event::RandomizeMinion(mouse_world_pos));
			} else {
				events.push(Event::NewMinion(mouse_world_pos));
			}
		}

		match input_state.dragging() {
			input::Dragging::Begin(_, from) => {
				let from = world_transform.to_world(from);
				events.push(Event::BeginDrag(from, from));
			}
			input::Dragging::Dragging(_, from, to) => {
				events.push(Event::Drag(
					world_transform.to_world(from),
					world_transform.to_world(to),
				));
			}
			input::Dragging::End(_, from, to, prev) => {
				let mouse_vel = (view_transform.to_view(prev) - to) / dt.into();
				events.push(Event::EndDrag(
					world_transform.to_world(from),
					world_transform.to_world(to),
					mouse_vel,
				));
			}
			_ => {}
		}
		(mouse_left_pressed, mouse_world_pos)
	}

	fn events_on_gamepad<I>(
		input_state: &I,
		events: &mut Vec<Event>,
		mouse_left_pressed: bool,
		mouse_world_pos: Position,
	) where
		I: input::InputRead,
	{
		let firerate = input_state.gamepad_axis(0, input::Axis::L2);
		let firepower = input_state.gamepad_axis(0, input::Axis::R2);
		if firepower >= DEAD_ZONE {
			events.push(Event::PrimaryTrigger(firepower, f64::from(firerate)));
		} else if input_state.key_pressed(input::Key::Space) || mouse_left_pressed {
			events.push(Event::PrimaryTrigger(1.0, 1.0));
		}
		let thrust = Position {
			x: if input_state.key_pressed(input::Key::Right) {
				1.
			} else if input_state.key_pressed(input::Key::Left) {
				-1.
			} else {
				input_state.gamepad_axis(0, input::Axis::LStickX)
			},

			y: if input_state.key_pressed(input::Key::Up) {
				1.
			} else if input_state.key_pressed(input::Key::Down) {
				-1.
			} else {
				input_state.gamepad_axis(0, input::Axis::LStickY)
			},
		};

		let yaw = Position {
			x: input_state.gamepad_axis(0, input::Axis::RStickX),
			y: input_state.gamepad_axis(0, input::Axis::RStickY),
		};

		use cgmath::InnerSpace;
		let magnitude = thrust.magnitude2();
		events.push(Event::VectorThrust(
			if magnitude >= DEAD_ZONE {
				Some(thrust / magnitude.max(1.))
			} else {
				None
			},
			if input_state.key_pressed(input::Key::PageUp) {
				VectorDirection::Turn(TURN_SPEED)
			} else if input_state.key_pressed(input::Key::PageDown) {
				VectorDirection::Turn(-TURN_SPEED)
			} else if yaw.magnitude() >= DEAD_ZONE {
				VectorDirection::Orientation(yaw)
			} else if mouse_left_pressed {
				VectorDirection::LookAt(mouse_world_pos)
			} else if thrust.magnitude2() > 0.1 {
				VectorDirection::FromVelocity
			} else {
				VectorDirection::None
			},
		));
	}
}

impl InputController for DefaultController {
	fn update<V, W, I>(input_state: &I, view_transform: &V, world_transform: &W, dt: Seconds) -> Vec<Event>
	where
		V: ViewTransform,
		W: WorldTransform,
		I: input::InputRead, {
		let mut events = Vec::new();

		DefaultController::events_on_key_held(input_state, &mut events);
		DefaultController::events_on_key_pressed_once(input_state, &mut events);
		let (mouse_left_pressed, mouse_world_pos) =
			DefaultController::events_on_mouse_move(input_state, &mut events, view_transform, world_transform, dt);
		DefaultController::events_on_gamepad(input_state, &mut events, mouse_left_pressed, mouse_world_pos);

		events
	}
}
