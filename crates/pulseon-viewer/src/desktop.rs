use std::path::PathBuf;

use gpui::{
    App, Application, Bounds, Context, FocusHandle, KeyBinding, Menu, MenuItem, PathPromptOptions,
    Render, SharedString, SystemMenuType, Window, WindowBounds, WindowOptions, actions, div,
    prelude::*, px, rgb, size,
};
use pulseon_model::alignment::AlignmentAxis;
use pulseon_viewer::core::ViewerCore;
use pulseon_viewer::model::DiscoveryRequest;
use pulseon_viewer::worker::{Generation, ReadKind, ReadRequest, ReadWorker};

actions!(
    pulseon_viewer,
    [OpenProject, Refresh, ResetView, UseStep, UseElapsed, Quit]
);

pub fn run(project_path: Option<PathBuf>) {
    Application::new().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("cmd-o", OpenProject, None),
            KeyBinding::new("cmd-r", Refresh, None),
            KeyBinding::new("cmd-0", ResetView, None),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.set_menus(menus());
        let bounds = Bounds::centered(None, size(px(1_200.), px(800.)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..WindowOptions::default()
            },
            move |window, cx| cx.new(|cx| ViewerApp::new(project_path, window, cx)),
        );
        if let Err(error) = result {
            eprintln!("failed to open pulseon-viewer window: {error}");
            cx.quit();
        } else {
            cx.activate(true);
        }
    });
}

fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "PulseOn".into(),
            items: vec![
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit PulseOn Viewer", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("Open Project…", OpenProject),
                MenuItem::action("Refresh", Refresh),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Reset View", ResetView),
                MenuItem::separator(),
                MenuItem::action("Step", UseStep),
                MenuItem::action("Elapsed", UseElapsed),
            ],
        },
    ]
}

struct ViewerApp {
    focus: FocusHandle,
    source_path: Option<PathBuf>,
    worker: Option<ReadWorker>,
    core: ViewerCore,
    next_generation: u64,
    local_error: Option<String>,
}

impl ViewerApp {
    fn new(project_path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);
        let mut app = Self {
            focus,
            source_path: None,
            worker: None,
            core: ViewerCore::default(),
            next_generation: 1,
            local_error: None,
        };
        if let Some(path) = project_path {
            app.open_source(path);
        }
        app
    }

    fn open_source(&mut self, path: PathBuf) {
        self.worker = None;
        self.core.reset_source();
        self.local_error = None;
        self.source_path = Some(path.clone());
        match ReadWorker::spawn(&path) {
            Ok(worker) => {
                self.worker = Some(worker);
                self.refresh_catalog();
            }
            Err(error) => self.local_error = Some(error.to_string()),
        }
    }

    fn refresh_catalog(&mut self) {
        let selection = self.core.selection();
        self.submit(ReadRequest::Discover(DiscoveryRequest {
            project_id: selection.project_id.clone(),
            selected_run_ids: selection.run_ids.clone(),
        }));
    }

    fn submit(&mut self, request: ReadRequest) {
        let Some(worker) = self.worker.as_ref() else {
            return;
        };
        let generation = Generation(self.next_generation);
        self.next_generation = self.next_generation.saturating_add(1);
        match worker.submit(generation, request.clone()) {
            Ok(()) => self.core.begin(generation, &request),
            Err(error) => self.local_error = Some(error.to_string()),
        }
    }

    fn drain_events(&mut self) {
        while let Some(event) = self.worker.as_ref().and_then(ReadWorker::try_event) {
            self.core.apply(event);
        }
    }

    fn open_picker(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(SharedString::from("Open Project")),
        });
        cx.spawn(async move |this, cx| {
            let result = prompt.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            this.open_source(path);
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => this.local_error = Some(error.to_string()),
                    Err(error) => this.local_error = Some(error.to_string()),
                }
                cx.notify();
            })
        })
        .detach();
    }

    fn error(&self) -> Option<&str> {
        self.local_error
            .as_deref()
            .or_else(|| self.core.last_error())
    }

    fn status(&self) -> SharedString {
        if self.source_path.is_none() {
            return "Open a local PulseOn project to compare Runs.".into();
        }
        if self.core.catalog().is_none() && self.core.is_pending(ReadKind::Catalog) {
            return "Loading Projects…".into();
        }
        let Some(catalog) = self.core.catalog() else {
            return "No catalog is available.".into();
        };
        if catalog.projects.is_empty() {
            "This store does not contain any Projects.".into()
        } else {
            "Select a Project to begin.".into()
        }
    }

    fn on_open(&mut self, _: &OpenProject, _: &mut Window, cx: &mut Context<Self>) {
        self.open_picker(cx);
    }

    fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.local_error = None;
        self.refresh_catalog();
        cx.notify();
    }

    fn on_reset(&mut self, _: &ResetView, _: &mut Window, cx: &mut Context<Self>) {
        self.core.reset_view();
        cx.notify();
    }

    fn on_step(&mut self, _: &UseStep, _: &mut Window, cx: &mut Context<Self>) {
        self.core.select_axis(AlignmentAxis::Step);
        cx.notify();
    }

    fn on_elapsed(&mut self, _: &UseElapsed, _: &mut Window, cx: &mut Context<Self>) {
        self.core.select_axis(AlignmentAxis::ElapsedTime);
        cx.notify();
    }
}

impl Render for ViewerApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.drain_events();
        if [ReadKind::Catalog, ReadKind::Overview, ReadKind::Detail]
            .into_iter()
            .any(|kind| self.core.is_pending(kind))
        {
            window.request_animation_frame();
        }
        let source = self.source_path.as_ref().map_or_else(
            || "No project open".to_owned(),
            |path| path.display().to_string(),
        );
        let error = self.error().map(ToOwned::to_owned);

        div()
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_open))
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(Self::on_reset))
            .on_action(cx.listener(Self::on_step))
            .on_action(cx.listener(Self::on_elapsed))
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0xf7f7f8))
            .text_color(rgb(0x202124))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(rgb(0xd8dadd))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("PulseOn Viewer"),
                    )
                    .child(div().text_sm().text_color(rgb(0x6b7280)).child(source)),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .child(self.status())
                    .children(error.map(|message| {
                        div()
                            .max_w(px(640.))
                            .px_4()
                            .py_3()
                            .rounded_md()
                            .bg(rgb(0xfee2e2))
                            .text_color(rgb(0x991b1b))
                            .child(message)
                    }))
                    .child(
                        div()
                            .id("open-project")
                            .cursor_pointer()
                            .px_4()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x2563eb))
                            .text_color(rgb(0xffffff))
                            .on_click(cx.listener(|this, _, _, cx| this.open_picker(cx)))
                            .child("Open Project…"),
                    ),
            )
    }
}
