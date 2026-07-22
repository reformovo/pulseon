use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::{cell::RefCell, rc::Rc};

use gpui::{
    App, Application, Bounds, Context, FocusHandle, KeyBinding, KeyDownEvent, Menu, MenuItem,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathPromptOptions, Render,
    ScrollWheelEvent, SharedString, SystemMenuType, Task, Timer, Window, WindowBounds,
    WindowOptions, actions, div, prelude::*, px, rgb, size, uniform_list,
};
use pulseon_model::alignment::AlignmentAxis;
use pulseon_model::comparison::{EvidenceCompleteness, EvidenceReason};
use pulseon_model::metric::MetricKey;
use pulseon_model::run::{Run, RunId, RunStatus};
use pulseon_model::types::ProjectId;
use pulseon_viewer::core::{ApplyOutcome, MAX_SELECTED_RUNS, ViewerCore, run_matches_filter};
use pulseon_viewer::model::{CatalogSnapshot, DiscoveryRequest};
use pulseon_viewer::query::{CurveSelection, DetailRequest, OverviewRequest};
use pulseon_viewer::worker::{
    Generation, ReadEvent, ReadEventReceiver, ReadKind, ReadRequest, ReadWorker,
};

mod renderer;

use renderer::{ChartAdapter, HoverPoint};

#[derive(Clone, Copy, Debug)]
enum DragGesture {
    BrushStart,
    BrushEnd,
    BrushWindow { last_axis: f64 },
    Detail { last_x: f64 },
}

#[derive(Default)]
struct RunListCache {
    runs: Rc<[Run]>,
}

impl RunListCache {
    fn rebuild(&mut self, catalog: Option<&CatalogSnapshot>, filter: &str) {
        self.runs = catalog.map_or_else(Rc::default, |catalog| {
            catalog
                .runs
                .iter()
                .filter(|run| run_matches_filter(run, filter))
                .cloned()
                .collect::<Vec<_>>()
                .into()
        });
    }

    fn shared(&self) -> Rc<[Run]> {
        Rc::clone(&self.runs)
    }
}

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
    run_list: RunListCache,
    source_path: Option<PathBuf>,
    worker: Option<ReadWorker>,
    event_task: Option<Task<()>>,
    core: ViewerCore,
    next_generation: u64,
    local_error: Option<String>,
    chart_adapter: Rc<RefCell<ChartAdapter>>,
    overview_revision: u64,
    detail_revision: u64,
    overview_width: u32,
    detail_width: u32,
    hover: Option<HoverPoint>,
    drag: Option<DragGesture>,
    zoom_token: u64,
}

impl ViewerApp {
    fn new(project_path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);
        let mut app = Self {
            focus,
            filter_focus: cx.focus_handle().tab_stop(true),
            run_filter: String::new(),
            run_list: RunListCache::default(),
            source_path: None,
            worker: None,
            event_task: None,
            core: ViewerCore::default(),
            next_generation: 1,
            local_error: None,
            chart_adapter: Rc::new(RefCell::new(ChartAdapter::default())),
            overview_revision: 0,
            detail_revision: 0,
            overview_width: 1_000,
            detail_width: 1_000,
            hover: None,
            drag: None,
            zoom_token: 0,
        };
        if let Some(path) = project_path {
            app.open_source(path, cx);
        }
        app
    }

    fn open_source(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.event_task = None;
        self.worker = None;
        self.core.reset_source();
        self.run_list = RunListCache::default();
        self.chart_adapter.borrow_mut().clear();
        self.hover = None;
        self.drag = None;
        self.local_error = None;
        self.source_path = Some(path.clone());
        match ReadWorker::spawn(&path) {
            Ok(mut worker) => {
                let Some(events) = worker.take_event_receiver() else {
                    self.local_error = Some("native read worker has no event stream".to_owned());
                    return;
                };
                self.worker = Some(worker);
                self.listen_for_events(events, cx);
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

    fn listen_for_events(&mut self, events: ReadEventReceiver, cx: &mut Context<Self>) {
        self.event_task = Some(cx.spawn(async move |this, cx| {
            let mut events = events;
            loop {
                let (next_events, event) = cx
                    .background_spawn(async move {
                        let event = events.recv();
                        (events, event)
                    })
                    .await;
                events = next_events;
                let Ok(event) = event else {
                    break;
                };
                if this
                    .update(cx, |this, cx| {
                        this.apply_event(event);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn apply_event(&mut self, event: ReadEvent) {
        let kind = event.kind;
        let revision = event.generation.0;
        let succeeded = event.result.is_ok();
        if self.core.apply(event) != ApplyOutcome::Applied || !succeeded {
            return;
        }
        if kind == ReadKind::Catalog {
            self.run_list.rebuild(self.core.catalog(), &self.run_filter);
        }
        match kind {
            ReadKind::Catalog if self.curve_selection().is_some() => self.request_overview(),
            ReadKind::Overview => {
                self.overview_revision = revision;
                self.request_detail();
            }
            ReadKind::Detail => self.detail_revision = revision,
            ReadKind::Catalog => {}
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
            physical_width: self.overview_width,
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
            physical_width: self.detail_width,
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
                    Ok(Ok(paths)) => {
                        if let Some(path) = picked_directory(paths) {
                            this.open_source(path, cx);
                        }
                    }
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
        self.run_list.rebuild(self.core.catalog(), &self.run_filter);
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
        self.run_list.rebuild(self.core.catalog(), &self.run_filter);
        cx.stop_propagation();
        cx.notify();
    }

    fn render_workspace(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let catalog = self
            .core
            .catalog()
            .expect("catalog checked before rendering");
        let projects = catalog.projects.clone();
        let runs = self.run_list.shared();
        let metrics = catalog.metric_keys.clone();
        let selection = self.core.selection().clone();
        let filter_focus = self.filter_focus.clone();
        let selected_count = selection.run_ids.len();
        let main = self.render_detail(cx);

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
            .child(div().flex_1().h_full().overflow_hidden().child(main))
    }

    fn render_detail(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(snapshot) = self.core.detail_shared() else {
            let overview = self.core.overview_shared();
            let message =
                empty_detail_message(overview.is_some(), self.core.is_pending(ReadKind::Detail));
            return div()
                .size_full()
                .flex()
                .flex_col()
                .p_5()
                .gap_3()
                .children(
                    overview
                        .as_ref()
                        .map(|snapshot| self.render_legend(snapshot)),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(message),
                );
        };
        let selected = self.core.brush().map(|brush| brush.selected());
        let Some(viewport) = renderer::detail_viewport(&snapshot, selected) else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child("No drawable evidence is available in this viewport.");
        };
        let x_ticks = pulseon_chart_core::linear_ticks(viewport.x, 6);
        let y_ticks = pulseon_chart_core::linear_ticks(viewport.y, 6);
        let adapter = Rc::clone(&self.chart_adapter);
        let hit_snapshot = Arc::clone(&snapshot);
        let hit_adapter = Rc::clone(&self.chart_adapter);
        let axis = self.core.axis();
        let interaction_range = self.core.brush().map(|brush| brush.selected());
        let pending = self.core.is_pending(ReadKind::Detail)
            || interaction_range
                .is_some_and(|selected| !renderer::selection_covered(&snapshot, selected));

        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .p_5()
            .gap_3()
            .child(self.render_legend(&snapshot))
            .child(
                div()
                    .flex_1()
                    .min_h(px(240.))
                    .flex()
                    .child(
                        div()
                            .w(px(72.))
                            .h_full()
                            .flex()
                            .flex_col()
                            .justify_between()
                            .text_xs()
                            .text_color(rgb(0x6b7280))
                            .children(y_ticks.iter().rev().map(|value| format_tick(*value))),
                    )
                    .child(
                        div()
                            .id("detail-chart")
                            .relative()
                            .flex_1()
                            .h_full()
                            .border_1()
                            .border_color(rgb(0xd1d5db))
                            .bg(rgb(0xffffff))
                            .child(
                                renderer::detail_canvas(
                                    adapter,
                                    Arc::clone(&snapshot),
                                    self.detail_revision,
                                    viewport,
                                )
                                .size_full(),
                            )
                            .on_mouse_move(cx.listener(
                                move |this, event: &MouseMoveEvent, _, cx| {
                                    if event.dragging() {
                                        this.move_detail_drag(event, cx);
                                    } else {
                                        this.hover = hit_adapter.borrow().hit_test(
                                            &hit_snapshot,
                                            viewport,
                                            event.position,
                                        );
                                    }
                                    cx.notify();
                                },
                            ))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                    this.begin_detail_drag(event, cx);
                                }),
                            )
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.finish_drag(cx);
                                }),
                            )
                            .on_mouse_up_out(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.finish_drag(cx);
                                }),
                            )
                            .on_scroll_wheel(cx.listener(
                                |this, event: &ScrollWheelEvent, _, cx| {
                                    this.zoom_detail(event, cx);
                                },
                            ))
                            .on_hover(cx.listener(|this, hovered, _, cx| {
                                if !hovered {
                                    this.hover = None;
                                    cx.notify();
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .ml(px(72.))
                    .flex()
                    .justify_between()
                    .text_xs()
                    .text_color(rgb(0x6b7280))
                    .children(x_ticks.into_iter().map(format_tick)),
            )
            .child(
                div()
                    .ml(px(72.))
                    .text_center()
                    .text_sm()
                    .child(axis_label(axis)),
            )
            .child(self.render_overview(cx))
            .children(pending.then(|| {
                div()
                    .absolute()
                    .top(px(84.))
                    .right(px(28.))
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(0xfef3c7))
                    .text_sm()
                    .child("Updating viewport…")
            }))
            .children(self.hover.as_ref().map(|hover| {
                div()
                    .absolute()
                    .top(px(84.))
                    .left(px(100.))
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(0x111827))
                    .text_color(rgb(0xffffff))
                    .text_sm()
                    .child(format!("{} · {}", hover.run_name, hover.metric_key))
                    .child(format!(
                        "{}={} · step={} · value={}",
                        renderer::axis_value_label(axis),
                        hover.axis_value,
                        hover.step,
                        hover.value
                    ))
            }))
    }

    fn render_legend(&self, snapshot: &pulseon_viewer::query::CurveSnapshot) -> gpui::Div {
        div()
            .flex()
            .flex_wrap()
            .gap_3()
            .children(snapshot.series.iter().enumerate().map(|(index, curve)| {
                let drawable = matches!(
                    curve.evidence.completeness,
                    EvidenceCompleteness::Complete | EvidenceCompleteness::Partial
                );
                let color = if drawable {
                    renderer::series_color(index)
                } else {
                    rgb(0x9ca3af)
                };
                let evidence = format!(
                    "{:?}{}",
                    curve.evidence.completeness,
                    reasons_label(&curve.evidence.reasons)
                );
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().size(px(10.)).rounded_full().bg(color))
                    .child(curve.run.name.clone())
                    .child(div().text_xs().text_color(rgb(0x6b7280)).child(evidence))
            }))
    }

    fn render_overview(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(snapshot) = self.core.overview_shared() else {
            return div().h(px(96.));
        };
        let Some(brush) = self.core.brush() else {
            return div().h(px(96.));
        };
        let Some(viewport) = renderer::overview_viewport(&snapshot, brush) else {
            return div().h(px(96.));
        };
        let adapter = Rc::clone(&self.chart_adapter);
        let selected = brush.selected();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .id("overview-chart")
                    .h(px(96.))
                    .w_full()
                    .relative()
                    .cursor_pointer()
                    .border_1()
                    .border_color(rgb(0xd1d5db))
                    .bg(rgb(0xffffff))
                    .child(
                        renderer::overview_canvas(
                            adapter,
                            snapshot,
                            self.overview_revision,
                            viewport,
                            brush,
                        )
                        .size_full(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.begin_brush_drag(event, cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if event.dragging() {
                            this.move_brush_drag(event, cx);
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_drag(cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_drag(cx)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_xs()
                    .text_color(rgb(0x6b7280))
                    .child(format_tick(selected.start()))
                    .child(format_tick(selected.end())),
            )
    }

    fn begin_brush_drag(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(brush) = self.core.brush() else {
            return;
        };
        let adapter = self.chart_adapter.borrow();
        self.drag = match adapter.brush_target(brush, event.position) {
            Some(renderer::BrushDragTarget::Start) => Some(DragGesture::BrushStart),
            Some(renderer::BrushDragTarget::End) => Some(DragGesture::BrushEnd),
            Some(renderer::BrushDragTarget::Window) => adapter
                .overview_axis_at(brush, event.position)
                .map(|last_axis| DragGesture::BrushWindow { last_axis }),
            None => None,
        };
        self.hover = None;
        cx.notify();
    }

    fn move_brush_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(mut gesture) = self.drag else {
            return;
        };
        let Some(brush) = self.core.brush() else {
            return;
        };
        let Some(axis) = self
            .chart_adapter
            .borrow()
            .overview_axis_at(brush, event.position)
        else {
            return;
        };
        if let Some(brush) = self.core.brush_mut() {
            match &mut gesture {
                DragGesture::BrushStart => {
                    let _ = brush.resize_start(axis);
                }
                DragGesture::BrushEnd => {
                    let _ = brush.resize_end(axis);
                }
                DragGesture::BrushWindow { last_axis } => {
                    let _ = brush.pan_by(axis - *last_axis);
                    *last_axis = axis;
                }
                DragGesture::Detail { .. } => return,
            }
        }
        self.drag = Some(gesture);
        cx.notify();
    }

    fn begin_detail_drag(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(range) = self.core.brush().map(|brush| brush.selected()) else {
            return;
        };
        self.drag = self
            .chart_adapter
            .borrow()
            .detail_axis_at(range, event.position)
            .map(|_| DragGesture::Detail {
                last_x: f64::from(event.position.x),
            });
        self.hover = None;
        cx.notify();
    }

    fn move_detail_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(DragGesture::Detail { mut last_x }) = self.drag else {
            return;
        };
        let Some(range) = self.core.brush().map(|brush| brush.selected()) else {
            return;
        };
        let Some(delta) =
            self.chart_adapter
                .borrow()
                .detail_pan_delta(range, last_x, event.position)
        else {
            return;
        };
        if let Some(brush) = self.core.brush_mut() {
            let _ = brush.pan_by(delta);
        }
        last_x = f64::from(event.position.x);
        self.drag = Some(DragGesture::Detail { last_x });
        cx.notify();
    }

    fn finish_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            self.request_detail();
            cx.notify();
        }
    }

    fn zoom_detail(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let Some(range) = self.core.brush().map(|brush| brush.selected()) else {
            return;
        };
        let Some(anchor) = self
            .chart_adapter
            .borrow()
            .detail_axis_at(range, event.position)
        else {
            return;
        };
        let delta = f32::from(event.delta.pixel_delta(px(16.)).y);
        let factor = f64::from((-delta / 240.).exp().clamp(0.5, 2.));
        if self
            .core
            .brush_mut()
            .is_none_or(|brush| brush.zoom_at(anchor, factor).is_err())
        {
            return;
        }
        self.zoom_token = self.zoom_token.saturating_add(1);
        let token = self.zoom_token;
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_millis(100)).await;
            let _ = this.update(cx, |this, cx| {
                if this.zoom_token == token {
                    this.request_detail();
                    cx.notify();
                }
            });
        })
        .detach();
        cx.stop_propagation();
        cx.notify();
    }

    fn reconcile_canvas_widths(&mut self, scale_factor: f32) {
        let (overview, detail) = self.chart_adapter.borrow().physical_widths(scale_factor);
        let overview_changed = overview.is_some_and(|width| width != self.overview_width);
        let detail_changed = detail.is_some_and(|width| width != self.detail_width);
        if let Some(width) = overview {
            self.overview_width = width;
        }
        if let Some(width) = detail {
            self.detail_width = width;
        }
        if overview_changed {
            self.request_overview();
        }
        if detail_changed {
            self.request_detail();
        }
    }
}

impl Render for ViewerApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.reconcile_canvas_widths(window.scale_factor());
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
            .children(error.clone().map(error_banner))
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

fn error_banner(message: String) -> gpui::Div {
    div()
        .mx_5()
        .mt_3()
        .px_4()
        .py_3()
        .rounded_md()
        .bg(rgb(0xfee2e2))
        .text_color(rgb(0x991b1b))
        .child(message)
}

fn picked_directory(paths: Option<Vec<PathBuf>>) -> Option<PathBuf> {
    paths.and_then(|paths| paths.into_iter().next())
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
        AlignmentAxis::ElapsedTime => "Elapsed (ms)",
    }
}

fn format_tick(value: f64) -> String {
    if value.abs() >= 1_000_000. || (value != 0. && value.abs() < 0.001) {
        format!("{value:.2e}")
    } else if value.fract() == 0. {
        format!("{value:.0}")
    } else {
        format!("{value:.3}")
    }
}

const fn empty_detail_message(has_overview: bool, pending: bool) -> &'static str {
    if pending {
        "Loading curves…"
    } else if has_overview {
        "No drawable evidence is available for the selected Runs."
    } else {
        "Select Runs and one metric to draw curves."
    }
}

fn reasons_label(reasons: &[EvidenceReason]) -> String {
    if reasons.is_empty() {
        return String::new();
    }
    format!(
        " · {}",
        reasons
            .iter()
            .map(|reason| format!("{reason:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_cancellation_has_no_source_effect() {
        assert_eq!(picked_directory(None), None);
        assert_eq!(picked_directory(Some(Vec::new())), None);
    }

    #[test]
    fn picker_uses_the_single_selected_directory() {
        assert_eq!(
            picked_directory(Some(vec![PathBuf::from("project")])),
            Some(PathBuf::from("project"))
        );
    }

    #[test]
    fn empty_detail_message_distinguishes_evidence_from_missing_selection() {
        assert_eq!(
            empty_detail_message(true, false),
            "No drawable evidence is available for the selected Runs."
        );
        assert_eq!(
            empty_detail_message(false, false),
            "Select Runs and one metric to draw curves."
        );
        assert_eq!(empty_detail_message(true, true), "Loading curves…");
    }

    #[test]
    fn run_list_cache_shares_filtered_runs_between_renders() {
        let cache = RunListCache::default();

        let first = cache.shared();
        let second = cache.shared();

        assert!(Rc::ptr_eq(&first, &second));
    }
}
