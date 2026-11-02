use crate::{
    commands::{AnalysisCommand, UiCommand},
    controller::{UiController, UiSnapshot},
    theme,
};
use nsh_core::{element::input, prelude::*};
use std::{cell::RefCell, rc::Rc};

pub(crate) fn render(
    snapshot: UiSnapshot,
    controller: Rc<RefCell<UiController>>,
) -> impl IntoElement {
    let metrics_available = matches!(
        snapshot.state.scan.status,
        crate::state::ScanViewStatus::Complete
            | crate::state::ScanViewStatus::Partial
            | crate::state::ScanViewStatus::Canceled
    );
    let source_details = crate::pages::cleanup_sources::render(&snapshot, controller.clone());
    let source_categories = crate::pages::cleanup::categories(snapshot.clone(), controller.clone());
    let available = snapshot.state.scan.available_bytes;
    let capacity = snapshot.state.scan.total_bytes;
    let map_height = (window().height.0 - 370.0).clamp(240.0, 700.0);
    let map_width = (window().width.0 - 208.0 - 40.0 - 280.0 - 12.0).max(260.0);
    let analysis = snapshot.space_analysis;
    let page_number = analysis.page_number;
    let directory_page = analysis.page.clone();
    let directory_filter = analysis.filter.to_lowercase();
    let selected_node = analysis.selection.as_ref().and_then(|selection| {
        analysis
            .preview
            .nodes
            .iter()
            .find(|node| node.path == selection.path)
    });
    let selected_details = selected_node.map_or_else(String::new, |node| {
        format!(
            "逻辑 {} · 分配 {} · 修改 {}",
            crate::components::format_bytes(node.logical_size),
            crate::components::format_bytes(node.allocated_size),
            crate::components::format_modified(node.modified)
        )
    });
    let total_pages = analysis
        .page
        .total
        .div_ceil(crate::space_analysis::model::DIRECTORY_PAGE_SIZE);
    view()
        .flex_col()
        .gap_2()
        .label("Disktree 空间浏览器")
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    crate::components::action("返回上级", {
                        let c = controller.clone();
                        move || {
                            c.borrow_mut().dispatch(UiCommand::OpenAnalysisParent);
                        }
                    })
                    .disabled(analysis.breadcrumbs.len() <= 1),
                )
                .child(
                    text("空间分析")
                        .flex_1()
                        .text_size(px(18.0))
                        .text_color(theme::color(theme::TEXT)),
                )
                .child(crate::components::action("审查标记项目", {
                    let c = controller.clone();
                    move || {
                        c.borrow_mut()
                            .dispatch(UiCommand::Analysis(AnalysisCommand::ReviewMarks));
                    }
                })),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .children(analysis.breadcrumbs.iter().map(|(id, path)| {
                    let id = *id;
                    let c = controller.clone();
                    view()
                        .role(AccessibilityRole::Button)
                        .focusable(true)
                        .label(format!("导航 {path}"))
                        .on_press(move |_: &()| {
                            c.borrow_mut()
                                .dispatch(UiCommand::Analysis(AnalysisCommand::SetDirectory(id)));
                        })
                        .max_w(percentage(1.0))
                        .child(
                            text(
                                path.rsplit(['\\', '/'])
                                    .find(|part| !part.is_empty())
                                    .unwrap_or(path)
                                    .to_owned(),
                            )
                            .text_color(theme::color(theme::MUTED)),
                        )
                })),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .flex_wrap()
                .children(
                    [
                        ("逻辑大小", crate::space_analysis::model::Metric::Logical),
                        ("分配大小", crate::space_analysis::model::Metric::Allocated),
                        ("文件数量", crate::space_analysis::model::Metric::Files),
                    ]
                    .into_iter()
                    .map(|(label, metric)| {
                        let c = controller.clone();
                        view()
                            .role(AccessibilityRole::Button)
                            .focusable(true)
                            .label(label)
                            .disabled(!metrics_available)
                            .hint(if metrics_available {
                                "使用已有扫描数据，不会重新扫描"
                            } else {
                                "完成扫描后可切换权重"
                            })
                            .accessibility_state(move || AccessibilityState {
                                selected: analysis.metric == metric,
                                ..Default::default()
                            })
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .bg(theme::color(if analysis.metric == metric {
                                theme::SURFACE
                            } else {
                                theme::PANEL
                            }))
                            .on_press(move |_: &()| {
                                c.borrow_mut().dispatch(UiCommand::Analysis(
                                    AnalysisCommand::SetMetric(metric),
                                ));
                            })
                            .child(text(label).text_color(theme::color(theme::TEXT)))
                    }),
                )
                .child(text("视图深度").text_color(theme::color(theme::MUTED)))
                .children((1..=4).map(|depth| {
                    let c = controller.clone();
                    view()
                        .role(AccessibilityRole::Button)
                        .label(format!("深度 {depth}"))
                        .focusable(true)
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(theme::color(if analysis.depth == depth {
                            theme::SURFACE
                        } else {
                            theme::PANEL
                        }))
                        .accessibility_state(move || AccessibilityState {
                            selected: analysis.depth == depth,
                            ..Default::default()
                        })
                        .on_press(move |_: &()| {
                            c.borrow_mut()
                                .dispatch(UiCommand::Analysis(AnalysisCommand::SetDepth(depth)));
                        })
                        .child(text(depth.to_string()).text_color(theme::color(theme::TEXT)))
                })),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(
                    input()
                        .label("目录路径")
                        .key(format!("directory:{}", analysis.directory_path))
                        .default_value(analysis.directory_path.clone())
                        .text_color(theme::color(theme::TEXT))
                        .bg(theme::color(theme::SURFACE))
                        .min_h(px(32.0))
                        .px_2()
                        .placeholder("输入扫描索引中的目录路径")
                        .flex_1()
                        .on_submit({
                            let c = controller.clone();
                            move |path: &str| {
                                c.borrow_mut()
                                    .dispatch(UiCommand::OpenAnalysisDirectory(path.to_owned()));
                            }
                        }),
                )
                .child(
                    input()
                        .label("筛选目录")
                        .key(format!("filter:{}", analysis.directory_path))
                        .text_color(theme::color(theme::TEXT))
                        .bg(theme::color(theme::SURFACE))
                        .min_h(px(32.0))
                        .px_2()
                        .placeholder("输入名称并按 Enter 筛选")
                        .flex_1()
                        .on_submit({
                            let c = controller.clone();
                            move |filter: &str| {
                                c.borrow_mut().dispatch(UiCommand::Analysis(
                                    AnalysisCommand::SetFilter(filter.to_owned()),
                                ));
                            }
                        }),
                )
                .child(crate::components::action("放大", {
                    let c = controller.clone();
                    move || {
                        c.borrow_mut()
                            .dispatch(UiCommand::Analysis(AnalysisCommand::Zoom(true)));
                    }
                }))
                .child(crate::components::action("缩小", {
                    let c = controller.clone();
                    move || {
                        c.borrow_mut()
                            .dispatch(UiCommand::Analysis(AnalysisCommand::Zoom(false)));
                    }
                }))
                .child(crate::components::action("适应窗口", {
                    let c = controller.clone();
                    move || {
                        c.borrow_mut()
                            .dispatch(UiCommand::Analysis(AnalysisCommand::Fit));
                    }
                }))
                .child(
                    text(format!(
                        "缩放 {:.0}% · 标记 {} 项",
                        analysis.zoom * 100.0,
                        analysis.marked.len()
                    ))
                    .text_color(theme::color(theme::MUTED)),
                ),
        )
        .child(
            view().flex_row().flex_wrap().gap_3().children(
                [
                    crate::space_analysis::model::FileCategory::Code,
                    crate::space_analysis::model::FileCategory::Git,
                    crate::space_analysis::model::FileCategory::Cache,
                    crate::space_analysis::model::FileCategory::Build,
                    crate::space_analysis::model::FileCategory::Media,
                    crate::space_analysis::model::FileCategory::Other,
                ]
                .into_iter()
                .map(|category| {
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(
                            view()
                                .w(px(8.0))
                                .h(px(8.0))
                                .bg(rgb(category_color(category))),
                        )
                        .child(
                            text(category_name(category))
                                .text_size(px(11.0))
                                .text_color(theme::color(theme::MUTED)),
                        )
                }),
            ),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_3()
                .child(
                    view()
                        .relative()
                        .flex_1()
                        .min_w(px(260.0))
                        .h(px(map_height))
                        .overflow_hidden()
                        .label("空间图")
                        .on_pan_end({
                            let c = controller.clone();
                            move |e: &PanEvent| {
                                c.borrow_mut()
                                    .dispatch(UiCommand::Analysis(AnalysisCommand::Pan(
                                        e.translation.x.0 as i32,
                                        e.translation.y.0 as i32,
                                    )));
                            }
                        })
                        .bg(theme::color(theme::PANEL))
                        .children(
                            crate::space_analysis::layout::layout_nested_treemap(
                                &analysis.preview,
                                &analysis.directory_path,
                                map_width,
                                map_height,
                            )
                            .into_iter()
                            .map(|tile| {
                                let node = &analysis.preview.nodes[tile.node];
                                let marked = analysis.marked.contains(&node.path);
                                let path = node.path.clone();
                                let directory = node.directory;
                                let aggregate = node.aggregate;
                                let selection = crate::space_analysis::model::AnalysisSelection {
                                    path: node.path.clone(),
                                    directory,
                                    file_identity: node.file_identity,
                                };
                                let selected = analysis
                                    .selection
                                    .as_ref()
                                    .is_some_and(|value| value.path == node.path);
                                let c = controller.clone();
                                view()
                                    .absolute()
                                    .left(percentage(
                                        (tile.x * analysis.zoom + analysis.pan.0) / map_width,
                                    ))
                                    .top(px(tile.y * analysis.zoom + analysis.pan.1))
                                    .w(percentage(tile.width * analysis.zoom / map_width))
                                    .h(px((tile.height * analysis.zoom).max(1.0)))
                                    .overflow_hidden()
                                    .p_1()
                                    .border_1()
                                    .border_color(theme::color(if selected {
                                        theme::TEXT
                                    } else {
                                        theme::BACKGROUND
                                    }))
                                    .bg(theme::tile_color(node.category, marked))
                                    .role(if aggregate {
                                        AccessibilityRole::Text
                                    } else {
                                        AccessibilityRole::Button
                                    })
                                    .focusable(!aggregate)
                                    .label(node.name.clone())
                                    .on_press(move |_: &()| {
                                        if aggregate {
                                            return;
                                        }
                                        if selected && directory.is_some() {
                                            c.borrow_mut().dispatch(
                                                UiCommand::OpenAnalysisDirectory(
                                                    path.display().to_owned(),
                                                ),
                                            );
                                        } else {
                                            c.borrow_mut().dispatch(UiCommand::Analysis(
                                                AnalysisCommand::SetSelection(Some(
                                                    selection.clone(),
                                                )),
                                            ));
                                        }
                                    })
                                    .child(
                                        text(if aggregate {
                                            format!("{} · 聚合", node.name)
                                        } else {
                                            format!(
                                                "{}\n{}",
                                                node.name,
                                                crate::components::format_bytes(node.logical_size)
                                            )
                                        })
                                        .text_size(px(12.0))
                                        .text_color(theme::color(theme::TEXT)),
                                    )
                            }),
                        ),
                )
                .child(
                    scroll()
                        .label("目录侧栏")
                        .w(px(280.0))
                        .h(px(map_height))
                        .bg(theme::color(theme::PANEL))
                        .child(
                            view()
                                .flex_col()
                                .gap_2()
                                .p_3()
                                .label("目录详情")
                                .child(
                                    text("当前目录")
                                        .text_size(px(11.0))
                                        .text_color(theme::color(theme::MUTED)),
                                )
                                .child(
                                    text(crate::components::format_bytes(analysis.directory_bytes))
                                        .text_size(px(26.0))
                                        .text_color(theme::color(theme::TEXT)),
                                )
                                .child(
                                    text(analysis.selection.as_ref().map_or_else(
                                        || "未选择项目".to_owned(),
                                        |selection| selection.path.display().to_owned(),
                                    ))
                                    .text_color(theme::color(theme::TEXT)),
                                )
                                .child(
                                    text(analysis.selection.as_ref().map_or("", |selection| {
                                        if selection.directory.is_some() {
                                            "类型：目录"
                                        } else {
                                            "类型：文件"
                                        }
                                    }))
                                    .text_color(theme::color(theme::MUTED)),
                                )
                                .child(
                                    text(selected_details).text_color(theme::color(theme::MUTED)),
                                )
                                .child(
                                    text(format!(
                                        "目录项目 {}-{} / {}",
                                        if directory_page.total == 0 {
                                            0
                                        } else {
                                            directory_page.offset.saturating_add(1)
                                        },
                                        (directory_page.offset + directory_page.entries.len()),
                                        directory_page.total
                                    ))
                                    .text_color(theme::color(theme::MUTED)),
                                )
                                .child(
                                    text(format!("子目录 {} 个", analysis.child_directories.len()))
                                        .text_color(theme::color(theme::MUTED)),
                                )
                                .children(
                                    analysis
                                        .child_directories
                                        .into_iter()
                                        .chain(
                                            directory_page
                                                .entries
                                                .into_iter()
                                                .filter(|entry| entry.directory_id.is_none()),
                                        )
                                        .filter(|entry| {
                                            directory_filter.is_empty()
                                                || entry
                                                    .name
                                                    .to_lowercase()
                                                    .contains(&directory_filter)
                                        })
                                        .map(|entry| {
                                            let path = entry.path.clone();
                                            let directory = entry.directory_id;
                                            let marked = analysis.marked.contains(&entry.path);
                                            let c = controller.clone();
                                            view()
                                                .flex_row()
                                                .items_center()
                                                .justify_between()
                                                .px_3()
                                                .py_2()
                                                .bg(theme::color(if marked {
                                                    theme::SURFACE
                                                } else {
                                                    theme::PANEL
                                                }))
                                                .child(
                                                    text(format!(
                                                        "{}\n{} B / {} B · {}",
                                                        entry.name,
                                                        entry.logical_size,
                                                        entry.allocated_size,
                                                        crate::components::format_modified(
                                                            entry.last_write_time
                                                        )
                                                    ))
                                                    .flex_1()
                                                    .text_size(px(12.0))
                                                    .text_color(theme::color(theme::TEXT)),
                                                )
                                                .child(if directory.is_some() {
                                                    let c = controller.clone();
                                                    let path = entry.path.clone();
                                                    crate::components::action(
                                                        if marked {
                                                            "取消目录标记"
                                                        } else {
                                                            "标记目录"
                                                        },
                                                        move || {
                                                            c.borrow_mut().dispatch(
                                                                UiCommand::Analysis(
                                                                    AnalysisCommand::ToggleMark(
                                                                        path.clone(),
                                                                    ),
                                                                ),
                                                            );
                                                        },
                                                    )
                                                    .into_element()
                                                } else {
                                                    view().into_element()
                                                })
                                                .child(crate::components::action(
                                                    if directory.is_some() {
                                                        "打开"
                                                    } else if marked {
                                                        "取消标记"
                                                    } else {
                                                        "标记"
                                                    },
                                                    move || {
                                                        if directory.is_some() {
                                                            c.borrow_mut().dispatch(
                                                                UiCommand::OpenAnalysisDirectory(
                                                                    path.display().to_owned(),
                                                                ),
                                                            );
                                                        } else {
                                                            c.borrow_mut().dispatch(
                                                                UiCommand::Analysis(
                                                                    AnalysisCommand::ToggleMark(
                                                                        path.clone(),
                                                                    ),
                                                                ),
                                                            );
                                                        }
                                                    },
                                                ))
                                        }),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .gap_2()
                                        .child(
                                            crate::components::action("上一页", {
                                                let c = controller.clone();
                                                move || {
                                                    c.borrow_mut().dispatch(UiCommand::Analysis(
                                                        AnalysisCommand::SetPage(
                                                            page_number.saturating_sub(1),
                                                        ),
                                                    ));
                                                }
                                            })
                                            .disabled(page_number == 0),
                                        )
                                        .child(
                                            text(format!(
                                                "第 {} / {} 页",
                                                page_number + 1,
                                                total_pages.max(1)
                                            ))
                                            .text_color(theme::color(theme::MUTED)),
                                        )
                                        .child(
                                            crate::components::action("下一页", {
                                                let c = controller.clone();
                                                move || {
                                                    c.borrow_mut().dispatch(UiCommand::Analysis(
                                                        AnalysisCommand::SetPage(
                                                            (page_number + 1)
                                                                .min(total_pages.saturating_sub(1)),
                                                        ),
                                                    ));
                                                }
                                            })
                                            .disabled(page_number + 1 >= total_pages),
                                        ),
                                ),
                        ),
                ),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .justify_between()
                .gap_2()
                .child(
                    text(format!(
                        "可用 {} / 总容量 {}",
                        crate::components::format_bytes(available),
                        crate::components::format_bytes(capacity)
                    ))
                    .text_size(px(12.0))
                    .text_color(theme::color(theme::MUTED)),
                )
                .child(
                    text(format!(
                        "缩放 {:.0}% · 标记 {} 项",
                        analysis.zoom * 100.0,
                        analysis.marked.len()
                    ))
                    .text_size(px(12.0))
                    .text_color(theme::color(theme::MUTED)),
                ),
        )
        .child(source_details)
        .child(source_categories)
}

fn category_color(category: crate::space_analysis::model::FileCategory) -> u32 {
    use crate::space_analysis::model::FileCategory::*;
    match category {
        Code => 0x2563eb,
        Git => 0x7c3aed,
        Cache => 0xd97706,
        Build => 0xdc2626,
        Toolchain => 0x0891b2,
        Media => 0x059669,
        Document => 0x4f46e5,
        Archive => 0xc2410c,
        Other => 0x52525b,
    }
}

fn category_name(category: crate::space_analysis::model::FileCategory) -> &'static str {
    use crate::space_analysis::model::FileCategory::*;
    match category {
        Code => "代码",
        Git => "版本库",
        Cache => "缓存",
        Build => "构建",
        Toolchain => "工具链",
        Media => "媒体",
        Document => "文档",
        Archive => "压缩包",
        Other => "其他",
    }
}
