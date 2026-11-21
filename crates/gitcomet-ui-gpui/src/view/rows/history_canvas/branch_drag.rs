//! Precise branch-chip dragging. Dropping only opens the shared branch menu.

use super::*;
use crate::view::rows::branch_drag::{BranchDrag, BranchDragPreview};
use std::rc::Rc;

#[derive(Clone)]
struct Press {
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    target: BranchMenuTarget,
    lifetime: u64,
}

fn live_refs(
    view: &HistoryView,
    repo_id: RepoId,
    lifetime: u64,
    targets: &[&BranchMenuTarget],
) -> bool {
    if !view
        .active_repo()
        .is_some_and(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
    {
        return false;
    }
    let Some(state) = view.store.snapshot() else {
        return false;
    };
    BranchDrag::live_refs(&state, repo_id, lifetime, targets)
}

fn branch_at(
    targets: &HistoryRefHitTargets,
    position: Point<Pixels>,
) -> Option<(Bounds<Pixels>, BranchMenuTarget)> {
    targets.refs.iter().find_map(|(bounds, target)| {
        if bounds.contains(&position)
            && let HistoryMenuRef::Branch(target) = target
        {
            Some((*bounds, target.clone()))
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_hit_testing_ignores_tags_overflow_and_clipped_away_pixels() {
        let tag = Bounds::new(point(px(0.0), px(0.0)), size(px(20.0), px(20.0)));
        let visible = Bounds::new(point(px(30.0), px(0.0)), size(px(8.0), px(20.0)));
        let overflow = Bounds::new(point(px(50.0), px(0.0)), size(px(20.0), px(20.0)));
        let local = BranchMenuTarget::local("a");
        let targets = HistoryRefHitTargets {
            refs: vec![
                (tag, HistoryMenuRef::Tag("a".into())),
                (visible, HistoryMenuRef::Branch(local.clone())),
            ],
            overflow: Some(overflow),
            ..Default::default()
        };
        assert_eq!(
            branch_at(&targets, visible.center()),
            Some((visible, local))
        );
        assert_eq!(branch_at(&targets, tag.center()), None);
        assert_eq!(branch_at(&targets, overflow.center()), None);
        assert_eq!(branch_at(&targets, point(px(40.0), px(10.0))), None);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_and_register(
    view: Entity<HistoryView>,
    repo_id: RepoId,
    theme: AppTheme,
    hitbox: &gpui::Hitbox,
    element_id: &gpui::GlobalElementId,
    targets: Rc<RefCell<HistoryRefHitTargets>>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(lifetime) = view
        .read(cx)
        .active_repo()
        .filter(|repo| repo.id == repo_id)
        .map(RepoState::lifetime)
    else {
        return;
    };
    let pending = window.with_element_state(
        element_id,
        |pending: Option<Rc<RefCell<Option<Press>>>>, _| {
            let pending = pending.unwrap_or_default();
            (pending.clone(), pending)
        },
    );
    if hitbox.is_hovered(window)
        && let Some(drag) = cx
            .active_drag()
            .and_then(|drag| drag.value.downcast_ref::<BranchDrag>())
        && let Some((bounds, target)) = branch_at(&targets.borrow(), window.mouse_position())
        && drag.can_drop_on(repo_id, &target)
        && drag.lifetime == lifetime
        && live_refs(view.read(cx), repo_id, lifetime, &[&drag.target, &target])
    {
        window.paint_quad(
            PaintQuad::new(bounds)
                .background(gpui::rgba(0x00000000))
                .border_widths(px(1.0))
                .border_color(theme.colors.accent.foreground),
        );
    }
    window.on_mouse_event({
        let view = view.clone();
        let hitbox = hitbox.clone();
        let targets = targets.clone();
        let pending = pending.clone();
        move |event: &gpui::MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture {
                return;
            }
            if event.button != MouseButton::Left {
                return;
            }
            if view.read(cx).history_branch_drag_click_suppressed {
                view.update(cx, |view, _| {
                    view.history_branch_drag_click_suppressed = false
                });
            }
            *pending.borrow_mut() = None;
            targets.borrow_mut().released_branch = None;
            if event.modifiers.number_of_modifiers() == 0
                && !cx.has_active_drag()
                && hitbox.is_hovered(window)
                && let Some((bounds, target)) = branch_at(&targets.borrow(), event.position)
                && live_refs(view.read(cx), repo_id, lifetime, &[&target])
            {
                *pending.borrow_mut() = Some(Press {
                    position: event.position,
                    bounds,
                    target,
                    lifetime,
                });
            }
        }
    });
    window.on_mouse_event({
        let view = view.clone();
        let pending = pending.clone();
        let targets = targets.clone();
        move |event: &gpui::MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if event.pressed_button != Some(MouseButton::Left)
                || event.modifiers.number_of_modifiers() != 0
            {
                pending.borrow_mut().take();
                return;
            }
            if cx.has_active_drag() {
                return;
            }
            let Some(press) = pending.borrow().clone() else {
                return;
            };
            if press.lifetime != lifetime
                || !live_refs(view.read(cx), repo_id, lifetime, &[&press.target])
            {
                pending.borrow_mut().take();
                return;
            }
            // GPUI Div uses the same 2 logical-pixel drag threshold.
            if (event.position - press.position).magnitude() <= 2.0 {
                return;
            }
            pending.borrow_mut().take();
            // A refresh can replace a ref while a pointer is held down.
            if !targets.borrow().refs.iter().any(|(bounds, target)| {
                bounds.contains(&press.position)
                    && target == &HistoryMenuRef::Branch(press.target.clone())
            }) {
                return;
            }
            let drag = BranchDrag {
                repo_id,
                lifetime,
                target: press.target,
            };
            let offset = event.position - press.bounds.origin;
            let preview = cx.new(|_| BranchDragPreview::new(&drag, theme, offset));
            cx.start_drag(gpui::AnyDrag::new(drag, preview).offset(offset));
            view.update(cx, |view, cx| view.begin_history_branch_drag(cx));
            window.refresh();
            cx.stop_propagation();
        }
    });
    window.on_mouse_event({
        let view = view.clone();
        let hitbox = hitbox.clone();
        move |event: &gpui::MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }
            let press = pending.borrow_mut().take();
            if !hitbox.is_hovered(window) {
                return;
            }
            if matches!(event.click_count, 1 | 2)
                && event.modifiers.number_of_modifiers() == 0
                && !cx.has_active_drag()
                && let Some(press) = press
                && press.lifetime == lifetime
            {
                let mut targets = targets.borrow_mut();
                // The label beneath the original coordinates may have moved
                // during a repaint. Keep the identity captured on mouse-down.
                if branch_at(&targets, event.position)
                    .is_some_and(|(_, target)| target == press.target)
                    && live_refs(view.read(cx), repo_id, lifetime, &[&press.target])
                {
                    targets.released_branch = Some((press.position, lifetime, press.target));
                }
            }
            let Some(drag) = cx
                .active_drag()
                .and_then(|drag| drag.value.downcast_ref::<BranchDrag>())
                .cloned()
            else {
                return;
            };
            let target = branch_at(&targets.borrow(), event.position)
                .map(|(_, target)| target)
                .filter(|target| {
                    event.modifiers.number_of_modifiers() == 0
                        && drag.can_drop_on(repo_id, target)
                        && drag.lifetime == lifetime
                        && live_refs(view.read(cx), repo_id, lifetime, &[&drag.target, target])
                });
            cx.stop_drag(window);
            if let Some(target) = target {
                view.update(cx, |view, cx| {
                    view.open_popover_at(
                        PopoverKind::BranchDropMenu {
                            repo_id,
                            source: drag.target,
                            target,
                        },
                        event.position,
                        window,
                        cx,
                    );
                });
            }
            window.refresh();
            cx.stop_propagation();
        }
    });
}
