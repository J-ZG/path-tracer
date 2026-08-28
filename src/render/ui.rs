//! Functions for the UI.
//!
//! Only used within [`crate::State::new()`] and [`crate::State::update()`].

use super::scene;

/// Define the look of the UI.
pub fn style() -> egui::Style {
    use egui::{
        Color32, CornerRadius, CursorIcon, FontId, Margin, Shadow, Stroke, Style, TextStyle, vec2,
    };

    let mut style = Style::default();

    style.spacing.item_spacing = vec2(8.0, 3.0);
    style.spacing.window_margin = Margin::same(8);
    style.spacing.slider_width = 80.0;
    style.spacing.slider_rail_height = 9.0;

    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(16.0)),
        (TextStyle::Body, FontId::proportional(13.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
        (TextStyle::Button, FontId::proportional(13.0)),
    ]
    .into();

    style.animation_time = 0.0;

    let visuals = &mut style.visuals;

    visuals.weak_text_color = Some(Color32::RED); // ?

    visuals.faint_bg_color = Color32::RED; // ?
    visuals.extreme_bg_color = Color32::TRANSPARENT; // scrollbar background

    visuals.text_edit_bg_color = Some(Color32::BLACK); // text editing background

    visuals.code_bg_color = Color32::RED;

    visuals.window_corner_radius = CornerRadius::same(4);
    visuals.window_shadow = Shadow {
        offset: [0, 0],
        blur: 20,
        spread: 0,
        color: Color32::from_black_alpha(40),
    };
    visuals.window_fill = Color32::from_black_alpha(140);
    visuals.window_stroke = Stroke::new(1.0, Color32::from_white_alpha(60));

    visuals.menu_corner_radius = CornerRadius::same(4); // color selection corner radius

    visuals.panel_fill = Color32::RED;

    visuals.popup_shadow = Shadow {
        offset: [0, 0],
        blur: 0,
        spread: 0,
        color: Color32::BLACK,
    }; // color selection shadow

    visuals.button_frame = true; // whether to show background and border for slider textbox

    visuals.collapsing_header_frame = false; // ?

    visuals.window_highlight_topmost = false;

    visuals.interact_cursor = Some(CursorIcon::PointingHand);

    let widgets = &mut visuals.widgets;

    widgets.noninteractive.bg_fill = Color32::RED;
    widgets.noninteractive.weak_bg_fill = Color32::RED;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(60)); // separator
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_white_alpha(220)); // text color
    widgets.noninteractive.corner_radius = CornerRadius::same(2);

    widgets.inactive.bg_fill = Color32::from_white_alpha(30); // slider background, slider nub background, color selection border, not hovered
    widgets.inactive.weak_bg_fill = Color32::from_black_alpha(60); // slider textbox background not hovered
    widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(30)); // slider textbox border not hovered
    widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_white_alpha(220)); // slider textbox text, slider nub border, not hovered
    widgets.inactive.corner_radius = CornerRadius::same(2);

    widgets.hovered.bg_fill = Color32::from_white_alpha(120); // color selection border, slider nub background, hovered
    widgets.hovered.weak_bg_fill = Color32::TRANSPARENT; // slider textbox background hovered
    widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::WHITE); // slider textbox border hovered
    widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE); // slider textbox text, slider nub border, hovered
    widgets.hovered.corner_radius = CornerRadius::same(2);

    widgets.active.bg_fill = Color32::from_white_alpha(180); // color selection border, slider nub background, clicked
    widgets.active.weak_bg_fill = Color32::BLACK; // slider textbox background clicked
    widgets.active.bg_stroke = Stroke::new(1.0, Color32::WHITE); // slider textbox border clicked
    widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE); // slider textbox text, slider nub border, clicked
    widgets.active.corner_radius = CornerRadius::same(2);

    widgets.open.bg_fill = Color32::WHITE; // color selection border while open
    widgets.open.weak_bg_fill = Color32::from_black_alpha(60); // window title bar, on top of the window
    widgets.open.bg_stroke = Stroke::new(1.0, Color32::RED);
    widgets.open.fg_stroke = Stroke::new(1.0, Color32::RED);
    widgets.open.corner_radius = CornerRadius::same(2);

    style
}

/// Draw text in the UI that look like a keycap (using [`egui::Frame`]).
fn key_label(ui: &mut egui::Ui, text: &str) {
    egui::Frame {
        inner_margin: egui::Margin {
            left: 2,
            right: 2,
            top: 0,
            bottom: 0,
        },
        fill: egui::Color32::from_white_alpha(20),
        stroke: egui::Stroke::new(1.0, egui::Color32::from_white_alpha(40)),
        corner_radius: egui::CornerRadius::same(2),
        outer_margin: egui::Margin::same(0),
        shadow: egui::Shadow::NONE,
    }
    .show(ui, |ui| ui.monospace(text));
}

/// What the user is doing in the UI, which [`run_base()`] returns.
pub enum BaseUiAction {
    UploadScene,
    UploadSkybox,
    UpdateSkyboxLuminance,
}

/// Run the base UI. Should be called inside the closure for [`egui::Context::run_ui()`].
///
/// `dt` is the change in time since the last frame in seconds, used to display the FPS in this UI.
/// `frame_count` is displayed in this UI as just `{frame_count} Frames`.
/// `upload_status`, if not empty, is displayed below the buttons for uploading files.
pub fn run_base(
    root_ui: &egui::Ui,
    scene: &mut scene::Scene,
    skybox: &mut scene::Skybox,
    dt: f32,
    frame_count: u32,
    upload_status: &str,
    is_hovering: bool,
) -> Vec<BaseUiAction> {
    // crosshair
    let raw_center = root_ui.available_rect_before_wrap().center();
    let center = egui::pos2(raw_center.x.round(), raw_center.y.round());
    let painter = root_ui.painter();
    let stroke = egui::Stroke::new(2.0, egui::Color32::WHITE);
    let dx = egui::vec2(8.0, 0.0);
    let dy = egui::vec2(0.0, 8.0);
    painter.line_segment([center - dx, center + dx], stroke);
    painter.line_segment([center - dy, center + dy], stroke);

    // frame count and FPS display
    egui::Area::new(egui::Id::new("Frame count and FPS")) // Area takes stuff inside it out of the root_ui layout flow
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-4.0, 0.0))
        .show(root_ui.ctx(), |ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                ui.monospace(format!("{frame_count} Frames\n{} FPS", dt.recip().round()))
            });
        });

    let mut actions = Vec::new();

    // hint and sliders
    egui::Area::new(egui::Id::new("Hint"))
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(0.0, 0.0))
        .show(root_ui.ctx(), |ui| {
            egui::Frame::window(ui.style())
                .outer_margin(egui::Margin::same(6)) // to match the Material Properties window default_pos
                .show(ui, |ui| {
                    ui.shrink_width_to_current(); // needed to shrink back when a wider collapsible is closed

                    let github_button_size = egui::vec2(18.0, 18.0);
                    let mut github_row_rect = egui::Rect::NOTHING;

                    ui.scope(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(3.0, 1.0);

                        ui.horizontal(|ui| {
                            ui.label("Enter / exit: Left click or");
                            key_label(ui, "Esc");
                        });

                        ui.horizontal(|ui| {
                            ui.label("Move:");
                            for key in ["W", "A", "S", "D", "Shift", "Space"] {
                                key_label(ui, key);
                            }
                            ui.add_space(github_button_size.x); // give some space for the GitHub button
                        });

                        github_row_rect = ui
                            .horizontal(|ui| {
                                ui.label("Toggle UI:");
                                key_label(ui, "F1");
                            })
                            .response
                            .rect;
                    });

                    let separator_y = ui.cursor().top();
                    ui.add_space(ui.spacing().item_spacing.y * 2.0);

                    ui.collapsing("Scene", |ui| {
                        if ui.button("Upload scene").clicked() {
                            actions.push(BaseUiAction::UploadScene);
                        }

                        if ui.button("Upload skybox").clicked() {
                            actions.push(BaseUiAction::UploadSkybox);
                        }

                        if !upload_status.is_empty() {
                            ui.label(upload_status);
                        }

                        let mut skybox_luminance = skybox.luminance;
                        if ui
                            .add(
                                egui::Slider::new(&mut skybox_luminance, 0.0..=20_000.0)
                                    .text("Skybox avg. nits"),
                            )
                            .changed()
                        {
                            skybox.luminance = skybox_luminance;
                            actions.push(BaseUiAction::UpdateSkyboxLuminance);
                        }
                    });

                    ui.collapsing("Camera", |ui| {
                        let pos = scene.camera.position();
                        ui.label(format!(
                            "({:.2}, {:.2}, {:.2})  ↔ {:.2}  ↕{:.2}",
                            pos.x,
                            pos.y,
                            pos.z,
                            scene.camera.yaw(),
                            scene.camera.pitch(),
                        ));

                        // fov
                        let mut fov_deg = scene.camera.fov_y().to_degrees();
                        let mut fov_slider = ui.add(
                            egui::Slider::new(&mut fov_deg, 20.0..=150.0)
                                .text("FOV")
                                .suffix("°"),
                        );
                        if fov_slider.drag_stopped_by(egui::PointerButton::Secondary)
                            || fov_slider.secondary_clicked()
                        {
                            fov_deg = scene.initial().camera.fov_y().to_degrees();
                            fov_slider.mark_changed();
                        }
                        if fov_slider.changed() {
                            scene.camera.set_fov(fov_deg.to_radians());
                        }

                        let speed_slider =
                            |ui: &mut egui::Ui, val: &mut f32, initial: f32, text: &str| -> bool {
                                let mut response = ui.add(
                                    egui::Slider::new(val, (initial * 0.01)..=(initial * 5.0))
                                        .text(text)
                                        .custom_formatter(move |n, _| {
                                            format!("{:.0}%", (n / f64::from(initial)) * 100.0)
                                        })
                                        .custom_parser(move |s| {
                                            s.trim_end_matches('%')
                                                .parse::<f64>()
                                                .ok()
                                                .map(|p| (p / 100.0) * f64::from(initial))
                                        }),
                                );
                                if response.drag_stopped_by(egui::PointerButton::Secondary)
                                    || response.secondary_clicked()
                                {
                                    *val = initial;
                                    response.mark_changed();
                                }
                                response.changed()
                            };

                        let initial_rot = scene.initial().camera.rotation_speeds[0];
                        if speed_slider(
                            ui,
                            &mut scene.camera.rotation_speeds[0],
                            initial_rot,
                            "Rotation Speed",
                        ) {
                            scene.camera.rotation_speeds[1] = scene.camera.rotation_speeds[0];
                        }

                        let initial_mov = scene.initial().camera.movement_speeds[0];
                        if speed_slider(
                            ui,
                            &mut scene.camera.movement_speeds[0],
                            initial_mov,
                            "Move Speed",
                        ) {
                            scene.camera.movement_speeds[1] = scene.camera.movement_speeds[0];
                        }
                    });

                    let github_button_rect = egui::Rect::from_center_size(
                        egui::pos2(
                            ui.min_rect().right() - (github_button_size.x / 2.0),
                            github_row_rect.center().y,
                        ),
                        github_button_size,
                    );

                    let github_button_response = ui.interact(
                        github_button_rect,
                        ui.id().with("Github Button"),
                        egui::Sense::click(),
                    );

                    ui.put(
                        github_button_rect,
                        egui::Image::new(egui::include_image!(
                            "../../assets/ui/GitHub_Invertocat_White.svg"
                        ))
                        .tint(if github_button_response.is_pointer_button_down_on() {
                            egui::Color32::from_gray(120)
                        } else if github_button_response.hovered() {
                            egui::Color32::from_gray(200)
                        } else {
                            egui::Color32::WHITE
                        })
                        .fit_to_exact_size(github_button_size)
                        .alt_text("GitHub"),
                    );

                    if github_button_response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }

                    if github_button_response.clicked() {
                        ui.ctx().open_url(egui::OpenUrl::same_tab(
                            "https://github.com/0dpe/path-tracer",
                        ));
                    }

                    ui.painter().hline(
                        ui.min_rect().x_range(),
                        separator_y + ui.spacing().item_spacing.y,
                        ui.visuals().widgets.noninteractive.bg_stroke,
                    );
                });
        });

    if is_hovering {
        let overlay_rect = root_ui.clip_rect(); // root_ui should cover the entire screen

        let painter = root_ui.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("File drop overlay"),
        ));

        painter.rect_filled(overlay_rect, 0.0, egui::Color32::from_black_alpha(150));

        painter.rect_stroke(
            overlay_rect.shrink(24.0),
            10.0,
            egui::Stroke::new(2.0, egui::Color32::WHITE),
            egui::StrokeKind::Inside,
        );

        painter.text(
            overlay_rect.center(),
            egui::Align2::CENTER_CENTER,
            "Drop file here",
            egui::FontId::proportional(30.0),
            egui::Color32::WHITE,
        );
    }

    actions
}

/// Run the UI for modifying a material. Should be called inside the closure for [`egui::Context::run_ui()`].
///
/// `material_index` should come from [`scene::Scene::takeout_gpu_camera()`] unwrapped.
///
/// Returns whether the user modified the material through this UI.
/// When `true` is returned, the user modified the material at the `material_index` index in [`scene::Scene::materials`], so this material has changed in [`scene::Scene`] already (as this function takes `&mut` to [`scene::Scene`]).
pub fn run_modify_material(
    root_ui: &egui::Ui,
    scene: &mut scene::Scene,
    material_index: usize,
) -> bool {
    let mut mat_changed = false;

    egui::Window::new(format!("Material {material_index}"))
        .id(egui::Id::new("Material Properties"))
        .movable(true)
        .collapsible(true)
        .default_pos(egui::pos2(6.0, 6.0))
        .default_width(305.0)
        .scroll(true)
        .order(egui::Order::Middle)
        .show(root_ui.ctx(), |ui| {
            let initial_mat = scene.initial().materials[material_index];
            let mat = &mut scene.materials[material_index];

            ui.horizontal(|ui| {
                ui.label("Right click to reset a slider.");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    if ui.button("Reset all").clicked() {
                        *mat = initial_mat;
                        mat_changed = true;
                    }
                });
            });

            ui.separator();

            let mat_slider = |ui: &mut egui::Ui,
                              val: &mut f32,
                              initial: f32,
                              range: std::ops::RangeInclusive<f32>,
                              text: &str|
             -> bool {
                let mut response = ui.add(egui::Slider::new(val, range).text(text));
                if response.drag_stopped_by(egui::PointerButton::Secondary)
                    || response.secondary_clicked()
                {
                    *val = initial;
                    response.mark_changed();
                }
                response.changed()
            };

            let color_rgba = |ui: &mut egui::Ui,
                              color: &mut glam::Vec4,
                              initial: glam::Vec4,
                              label: &str|
             -> bool {
                let mut arr = color.to_array();
                let mut changed = false;
                ui.horizontal(|ui| {
                    let mut btn = ui.color_edit_button_rgba_unmultiplied(&mut arr); // linear
                    if btn.secondary_clicked() {
                        arr = initial.to_array();
                        btn.mark_changed();
                    }
                    if btn.changed() {
                        *color = glam::Vec4::from_array(arr);
                        changed = true;
                    }
                    ui.label(label);
                });
                changed
            };

            let color_rgb = |ui: &mut egui::Ui,
                             color: &mut glam::Vec4,
                             initial: glam::Vec4,
                             label: &str|
             -> bool {
                let mut arr = [color.x, color.y, color.z];
                let mut changed = false;
                ui.horizontal(|ui| {
                    let mut btn = ui.color_edit_button_rgb(&mut arr); // linear
                    if btn.secondary_clicked() {
                        arr = [initial.x, initial.y, initial.z];
                        btn.mark_changed();
                    }
                    if btn.changed() {
                        color.x = arr[0];
                        color.y = arr[1];
                        color.z = arr[2];
                        changed = true;
                    }
                    ui.label(label);
                });
                changed
            };

            mat_changed |= mat_slider(
                ui,
                &mut mat.OpenPBR_base_weight,
                initial_mat.OpenPBR_base_weight,
                0.0..=1.0,
                "Base Weight",
            );
            mat_changed |= color_rgba(
                ui,
                &mut mat.base_color_factor,
                initial_mat.base_color_factor,
                "Base Color",
            );
            mat_changed |= mat_slider(
                ui,
                &mut mat.roughness_factor,
                initial_mat.roughness_factor,
                0.0..=1.0,
                "Base Diffuse Roughness",
            );
            mat_changed |= mat_slider(
                ui,
                &mut mat.metallic_factor,
                initial_mat.metallic_factor,
                0.0..=1.0,
                "Base Metalness",
            );

            mat_changed |= mat_slider(
                ui,
                &mut mat.specular_color_and_factor.w,
                initial_mat.specular_color_and_factor.w,
                0.0..=10.0,
                "Specular Weight",
            );
            mat_changed |= color_rgb(
                ui,
                &mut mat.specular_color_and_factor,
                initial_mat.specular_color_and_factor,
                "Specular Color",
            );

            mat_changed |= mat_slider(
                ui,
                &mut mat.OpenPBR_specular_roughness,
                initial_mat.OpenPBR_specular_roughness,
                0.0..=1.0,
                "Specular Roughness",
            );
            mat_changed |= mat_slider(ui, &mut mat.ior, initial_mat.ior, 0.0..=3.0, "Specular IOR");
            mat_changed |= mat_slider(
                ui,
                &mut mat.OpenPBR_specular_roughness_anisotropy,
                initial_mat.OpenPBR_specular_roughness_anisotropy,
                0.0..=1.0,
                "Specular Anisotropy",
            );
            mat_changed |= mat_slider(
                ui,
                &mut mat.OpenPBR_specular_haze,
                initial_mat.OpenPBR_specular_haze,
                0.0..=1.0,
                "Specular Haze",
            );
            mat_changed |= mat_slider(
                ui,
                &mut mat.OpenPBR_specular_haze_spread,
                initial_mat.OpenPBR_specular_haze_spread,
                0.0..=1.0,
                "Specular Haze Spread",
            );
            mat_changed |= mat_slider(
                ui,
                &mut mat.OpenPBR_specular_retroreflectivity,
                initial_mat.OpenPBR_specular_retroreflectivity,
                0.0..=1.0,
                "Specular Retroreflectivity",
            );

            mat_changed |= mat_slider(
                ui,
                &mut mat.OpenPBR_emission_weight,
                initial_mat.OpenPBR_emission_weight,
                0.0..=1.0,
                "Emission Weight",
            );
            mat_changed |= mat_slider(
                ui,
                &mut mat.emission_color_and_strength.w,
                initial_mat.emission_color_and_strength.w,
                0.0..=1_500_000.0,
                "Emission Luminance (nits)",
            );
            mat_changed |= color_rgb(
                ui,
                &mut mat.emission_color_and_strength,
                initial_mat.emission_color_and_strength,
                "Emission Color",
            );
            mat_changed |= mat_slider(
                ui,
                &mut mat.normal_scale,
                initial_mat.normal_scale,
                0.0..=10.0,
                "Normal Scale",
            );
        });

    mat_changed
}
