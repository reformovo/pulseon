use std::ops::Range;
use std::path::PathBuf;

use gpui::{
    App, Application, Bounds, Context, FocusHandle, KeyBinding, KeyDownEvent, Menu, MenuItem,
    PathPromptOptions, Render, SharedString, SystemMenuType, Window, WindowBounds, WindowOptions,
    actions, div, prelude::*, px, rgb, size, uniform_list,
};
use pulseon_model::alignment::AlignmentAxis;
use pulseon_model::metric::MetricKey;
use pulseon_model::run::{Run, RunId, RunStatus};
use pulseon_model::types::ProjectId;
use pulseon_viewer::core::{ApplyOutcome, MAX_SELECTED_RUNS, ViewerCore, run_matches_filter};
use pulseon_viewer::model::DiscoveryRequest;
use pulseon_viewer::query::{CurveSelection, DetailRequest, OverviewRequest};
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
    filter_focus: FocusHandle,
    run_filter: String,
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
            filter_focus: cx.focus_handle().tab_stop(true),
            run_filter: String::new(),
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
            let kind = event.kind;
            let succeeded = event.result.is_ok();
            if self.core.apply(event) != ApplyOutcome::Applied || !succeeded {
                continue;
            }
            match kind {
                ReadKind::Catalog if self.curve_selection().is_some() => self.request_overview(),
                ReadKind::Overview => self.request_detail(),
                ReadKind::Catalog | ReadKind::Detail => {}
            }
        }
    }

    fn curve_selection(&self) -> Option<CurveSelection> {
        let selection = self.core.selection();
        if selection.run_ids.is_empty() {
            return None;
        }
        Some(CurveSelection {
            run_ids: selection.run_ids.clone(),
            metric_key: selection.metric_key.clone()?,
            axis: self.core.axis(),
        })
    }

    fn request_overview(&mut self) {
        let Some(selection) = self.curve_selection() else {
            return;
        };
        self.submit(ReadRequest::Overview(OverviewRequest {
            selection,
            physical_width: 1_000,
        }));
    }

    fn request_detail(&mut self) {
        let Some(selection) = self.curve_selection() else {
            return;
        };
        let Some(viewport) = self.core.selected_viewport() else {
            return;
        };
        self.submit(ReadRequest::Detail(DetailRequest {
            selection,
            viewport,
            physical_width: 1_000,
        }));
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
        if self.core.reset_view() {
            self.request_detail();
        }
        cx.notify();
    }

    fn on_step(&mut self, _: &UseStep, _: &mut Window, cx: &mut Context<Self>) {
        self.core.select_axis(AlignmentAxis::Step);
        self.request_overview();
        cx.notify();
    }

    fn on_elapsed(&mut self, _: &UseElapsed, _: &mut Window, cx: &mut Context<Self>) {
        self.core.select_axis(AlignmentAxis::ElapsedTime);
        self.request_overview();
        cx.notify();
    }

    fn select_project(&mut self, project_id: ProjectId, cx: &mut Context<Self>) {
        self.core.select_project(Some(project_id));
        self.run_filter.clear();
        self.refresh_catalog();
        cx.notify();
    }

    fn toggle_run(&mut self, run_id: RunId, cx: &mut Context<Self>) {
        match self.core.toggle_run(run_id) {
            Ok(_) => {
                self.local_error = None;
                self.refresh_catalog();
            }
            Err(error) => self.local_error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn select_metric(&mut self, metric_key: MetricKey, cx: &mut Context<Self>) {
        self.core.select_metric(Some(metric_key));
        self.request_overview();
        cx.notify();
    }

    fn on_filter_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "backspace" => {
                self.run_filter.pop();
            }
            "escape" => self.run_filter.clear(),
            _ if !event.keystroke.modifiers.platform && !event.keystroke.modifiers.control => {
                if let Some(text) = event.keystroke.key_char.as_deref()
                    && !text.chars().any(char::is_control)
                {
                    self.run_filter.push_str(text);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn render_workspace(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let catalog = self
            .core
            .catalog()
            .expect("catalog checked before rendering");
        let projects = catalog.projects.clone();
        let runs: Vec<Run> = catalog
            .runs
            .iter()
            .filter(|run| run_matches_filter(run, &self.run_filter))
            .cloned()
            .collect();
        let metrics = catalog.metric_keys.clone();
        let selection = self.core.selection().clone();
        let filter_focus = self.filter_focus.clone();
        let selected_count = selection.run_ids.len();
        let main_status = selection.metric_key.as_ref().map_or_else(
            || "Select Runs and one metric to draw curves.".to_owned(),
            |metric| {
                format!(
                    "Preparing {} Run(s) · {} · {}",
                    selected_count,
                    metric.as_str(),
                    axis_label(self.core.axis())
                )
            },
        );

        div()
            .flex()
            .flex_1()
            .overflow_hidden()
            .child(
                div()
                    .w(px(360.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .border_r_1()
                    .border_color(rgb(0xd8dadd))
                    .child(section_label("Project"))
                    .children(projects.into_iter().enumerate().map(|(index, project)| {
                        let project_id = project.project_id.clone();
                        let selected = selection.project_id.as_ref() == Some(&project.project_id);
                        div()
                            .id(("project", index))
                            .cursor_pointer()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .when(selected, |row| row.bg(rgb(0xdbeafe)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_project(project_id.clone(), cx);
                            }))
                            .child(project.name)
                    }))
                    .when(selection.project_id.is_some(), |sidebar| {
                        sidebar
                            .child(section_label(&format!(
                                "Runs ({selected_count}/{MAX_SELECTED_RUNS})"
                            )))
                            .child(
                                div()
                                    .id("run-filter")
                                    .track_focus(&filter_focus)
                                    .cursor_text()
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(rgb(0xc7cbd1))
                                    .on_key_down(cx.listener(Self::on_filter_key))
                                    .on_click(move |_, window, _| filter_focus.focus(window))
                                    .child(if self.run_filter.is_empty() {
                                        "Filter by name, id, or status".to_owned()
                                    } else {
                                        self.run_filter.clone()
                                    }),
                            )
                            .child(
                                uniform_list(
                                    "runs",
                                    runs.len(),
                                    cx.processor(move |this, range: Range<usize>, _, cx| {
                                        range
                                            .filter_map(|index| {
                                                runs.get(index).map(|run| (index, run))
                                            })
                                            .map(|(index, run)| {
                                                let run_id = run.run_id.clone();
                                                let selected = this
                                                    .core
                                                    .selection()
                                                    .run_ids
                                                    .contains(&run.run_id);
                                                let can_toggle = selected
                                                    || this.core.selection().run_ids.len()
                                                        < MAX_SELECTED_RUNS;
                                                div()
                                                    .id(("run", index))
                                                    .h(px(52.))
                                                    .w_full()
                                                    .flex()
                                                    .flex_col()
                                                    .justify_center()
                                                    .px_3()
                                                    .border_b_1()
                                                    .border_color(rgb(0xe5e7eb))
                                                    .when(selected, |row| row.bg(rgb(0xecfdf5)))
                                                    .when(!can_toggle, |row| row.opacity(0.45))
                                                    .when(can_toggle, |row| {
                                                        row.cursor_pointer().on_click(cx.listener(
                                                            move |this, _, _, cx| {
                                                                this.toggle_run(run_id.clone(), cx);
                                                            },
                                                        ))
                                                    })
                                                    .child(run.name.clone())
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(rgb(0x6b7280))
                                                            .child(format!(
                                                                "{} · {}",
                                                                run.run_id.as_str(),
                                                                run_status(run.status)
                                                            )),
                                                    )
                                            })
                                            .collect::<Vec<_>>()
                                    }),
                                )
                                .h(px(300.)),
                            )
                            .child(section_label("Metric"))
                            .children(metrics.into_iter().enumerate().map(|(index, metric)| {
                                let selected = selection.metric_key.as_ref() == Some(&metric);
                                let selected_metric = metric.clone();
                                div()
                                    .id(("metric", index))
                                    .cursor_pointer()
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .when(selected, |row| row.bg(rgb(0xdbeafe)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_metric(selected_metric.clone(), cx);
                                    }))
                                    .child(metric.as_str().to_owned())
                            }))
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(main_status),
            )
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
        let has_catalog = self
            .core
            .catalog()
            .is_some_and(|catalog| !catalog.projects.is_empty());

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
            .child(if has_catalog {
                self.render_workspace(cx)
            } else {
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
                    )
            })
    }
}

fn section_label(label: &str) -> gpui::Div {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(0x6b7280))
        .child(label.to_owned())
}

const fn run_status(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "running",
        RunStatus::Finished => "finished",
        RunStatus::Failed => "failed",
    }
}

const fn axis_label(axis: AlignmentAxis) -> &'static str {
    match axis {
        AlignmentAxis::Step => "Step",
        AlignmentAxis::ElapsedTime => "Elapsed",
    }
}
