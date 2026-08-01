#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

// not supported because testing includes saving images to disk
#[cfg(all(feature = "testing", target_arch = "wasm32"))]
compile_error!("feature `testing` is not supported on wasm32");

// TODO: investigate why web performance is much worse than on native

pub mod render;
// pub for criterion benchmark since criterion is an external crate
use render::state::State;

mod logger;

/// The main function that runs the app, called by both native and web targets.
// wasm_bindgen(start) marks the first function that gets executed when this code, converted to wasm, is loaded into the browser
#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen(start))]
pub fn run() {
    logger::init_environment(log::LevelFilter::Info);
    log::info!("Logging enabled");

    // general style guide: all logging should be capitalized, while compile_error!() panic!() .expect() .context() should not be capitalized

    let event_loop = winit::event_loop::EventLoop::with_user_event()
        .build()
        .expect("event loop building failed"); // .expect() is called on both native and on web
    // there exists wasm_bindgen::UnwrapThrowExt::expect_throw and wasm_bindgen::UnwrapThrowExt::unwrap_throw,
    // but they are not used at all in this crate because they throw JS error directly, bypassing std::panic::set_hook(),
    // while in logger.rs, we actually set a panic_hook to allow us to implement some custom behavior on panic, like
    // logging to web console and sending a CustomEvent to JS

    // setting event loop control flow does not seem to make WindowEvents fire faster
    // triggering WindowEvent::RedrawRequested in a loop, redraws occur 60 times per second no matter if it's ControlFlow::Poll or ControlFlow::Wait
    // setting this elsewhere doesn't seem to make a difference either
    // event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

    #[cfg(not(target_arch = "wasm32"))]
    event_loop
        .run_app(&mut App::new())
        .expect("event loop run_app failed");
    // some guides use run_app() for web as well, however, this is not recommended by winit 0.30+ docs
    // if run_app() was used for web, there would be an intentional JavaScript control flow error in the web console

    #[cfg(target_arch = "wasm32")]
    {
        let app = App::new(&event_loop);
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(app);
    }
}

/// The main application struct that implements the [`winit::application::ApplicationHandler`] trait.
/// Holds the application state and, on wasm32, an event loop proxy for sending custom events.
struct App {
    #[cfg(target_arch = "wasm32")]
    proxy: Option<winit::event_loop::EventLoopProxy<State>>, // used to send custom events to EventLoop
    state: Option<State>,
}

impl App {
    #[allow(clippy::missing_const_for_fn)] // create_proxy() isn't const
    fn new(
        #[cfg(target_arch = "wasm32")] event_loop: &winit::event_loop::EventLoop<State>,
    ) -> Self {
        Self {
            #[cfg(target_arch = "wasm32")]
            proxy: Some(event_loop.create_proxy()), // returns EventLoopProxy
            state: None,
        }
    }
}

impl winit::application::ApplicationHandler<State> for App {
    /// Initialize the application by creating a window and a [`State`].
    // called by winit when the window is resumed
    // seems to be called only once when the window is opened
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        // resumed() has method signature that returns nothing, so anyhow cannot be used here

        #[allow(unused_mut)]
        // window_attributes doesn't have to be mut on non-wasm, but must be mut on wasm
        let mut window_attributes = winit::window::Window::default_attributes();

        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast; // needed for .dyn_into()
            // WindowAttributesExtWebSys is a trait implemented by winit::window::WindowAttributes used for .with_canvas()
            use winit::platform::web::WindowAttributesExtWebSys;

            const CANVAS_ID: &str = "rust-insert-wasm"; // matches with index.html <canvas id="rust-insert-wasm">

            window_attributes = window_attributes.with_canvas(Some(
                web_sys::window()
                    .expect("get web_sys window failed")
                    .document()
                    .expect("get web_sys document failed")
                    .get_element_by_id(CANVAS_ID)
                    .expect("canvas element not found in DOM")
                    .dyn_into::<web_sys::HtmlCanvasElement>()
                    .expect("canvas element found, but it was not an HtmlCanvasElement"),
            ));
        }

        let window = std::sync::Arc::new(
            event_loop
                .create_window(window_attributes)
                .expect("event loop create_window failed"),
        );
        window.set_title("Path Tracer");

        #[cfg(not(target_arch = "wasm32"))]
        {
            // use pollster async runtime/executor to await the future from State::new(window)
            self.state = Some(
                pollster::block_on(State::new(window, event_loop.owned_display_handle()))
                    .expect("State::new() failed"),
            );
            // for non-wasm, this is where self.state is set, and initialization is finished
        }

        #[cfg(target_arch = "wasm32")]
        {
            if let Some(proxy) = self.proxy.take() {
                let display_handle = event_loop.owned_display_handle();

                wasm_bindgen_futures::spawn_local(async move {
                    let state = State::new(window, display_handle)
                        .await
                        .expect("State::new() failed");

                    proxy
                        .send_event(state)
                        .unwrap_or_else(|_| panic!("send_event to EventLoopProxy failed"));
                    // .expect() isn't used here since the error EventLoopClosed<T> from send_event() doesn't implement Debug (would have to derive Debug for State)
                });
            }
        }
    }

    // called by winit when an event is sent from EventLoopProxy::send_event
    // EventLoopProxy<State>::send_event() is only called in resumed() under wasm32
    #[cfg(target_arch = "wasm32")]
    fn user_event(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, mut event: State) {
        // sometimes on initial web page load, neither render nor resize is called
        // so, force a resize call here, which triggers a render call too
        // on native, this doesn't seem to be necessary since a resize and render are both automatically called
        let size = event.window.inner_size();
        event.resize(size.width, size.height);
        self.state = Some(event); // for wasm, this is where self.state is set, and initialization is finished
    }

    /// Handle window events such as close, resize, redraw, keyboard input, and mouse button input by calling the corresponding methods in [`State`].
    // called by winit when the OS or browser sends an event to the window
    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        let Some(state) = &mut self.state else {
            return;
        };

        // let egui consume events only when the cursor isn't grabbed
        if !state.is_cursor_grabbed()
            && state
                .egui_winit_state
                .on_window_event(&state.window, &event)
                .consumed
        {
            return;
        }

        match event {
            winit::event::WindowEvent::CloseRequested => event_loop.exit(),
            winit::event::WindowEvent::Resized(size) => state.resize(size.width, size.height),
            winit::event::WindowEvent::RedrawRequested => state.update(),
            winit::event::WindowEvent::Focused(false) => state.release_cursor_grab(),
            winit::event::WindowEvent::KeyboardInput { event, .. } => state.key_event(&event),
            winit::event::WindowEvent::MouseInput {
                state: mouse_state,
                button,
                ..
            } => state.mouse_button_event(mouse_state, button),
            _ => {}
        }
    }

    /// Handle device events such as mouse motion by calling the corresponding method in [`State`].
    // device events are non-window specific, so they can be captured even when the window is not in focus
    // however, only when the mouse is over the window will mouse motion events be captured
    // mouse motion events for 3D camera control are better captured as device events
    fn device_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        if let Some(state) = &mut self.state
            && let winit::event::DeviceEvent::MouseMotion { delta } = event
        {
            state.mouse_move_event(delta);
        }
    }
}
