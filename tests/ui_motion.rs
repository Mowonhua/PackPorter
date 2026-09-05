//! 使用模拟时钟驱动真实 Slint 动画，验证退出生命周期与输入隔离，无需桌面或实际等待。

use i_slint_backend_testing::{init_no_event_loop, mock_elapsed_time};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};
use std::time::Duration;

slint::slint! {
    import { AnimatedLayer } from "../ui/motion.slint";

    export component MotionFixture inherits Window {
        width: 200px;
        height: 120px;
        in-out property <bool> shown;
        in-out property <bool> interactive: true;
        out property <float> progress: layer.progress;
        out property <bool> present: layer.present;
        out property <int> clicks;
        out property <int> keys;
        layer := AnimatedLayer {
            width: parent.width;
            height: parent.height;
            shown: root.shown;
            interactive: root.interactive;
            if layer.present: Rectangle {
                input := FocusScope {
                    key-pressed(event) => { root.keys += 1; return accept; }
                }
                TouchArea {
                    clicked => { root.clicks += 1; input.focus(); }
                }
            }
        }
    }
}

fn fixture() -> MotionFixture {
    init_no_event_loop();
    let ui = MotionFixture::new().expect("创建动画测试窗口失败");
    ui.show().expect("显示动画测试窗口失败");
    assert_eq!(ui.get_progress(), 0.0);
    assert!(!ui.get_present());
    ui
}

fn advance(milliseconds: u64) {
    mock_elapsed_time(Duration::from_millis(milliseconds));
}

fn click(ui: &MotionFixture) {
    let position = LogicalPosition::new(100.0, 60.0);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

fn type_key(ui: &MotionFixture) {
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: "a".into() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: "a".into() });
}

#[test]
fn layer_animates_in_and_remains_present_until_exit_finishes() {
    let ui = fixture();
    ui.set_shown(true);
    assert!(ui.get_present(), "打开请求应立即挂载内容");
    assert_eq!(ui.get_progress(), 0.0, "入场应从透明状态开始");
    advance(90);
    assert!(
        (0.0..1.0).contains(&ui.get_progress()),
        "入场中途应有可见的插值"
    );
    advance(100);
    assert_eq!(ui.get_progress(), 1.0);

    ui.set_shown(false);
    assert!(ui.get_present(), "关闭请求不能立即卸载，需保留退出动画");
    assert_eq!(ui.get_progress(), 1.0);
    advance(90);
    assert!((0.0..1.0).contains(&ui.get_progress()));
    assert!(ui.get_present());
    advance(100);
    assert_eq!(ui.get_progress(), 0.0);
    assert!(!ui.get_present(), "动画归零后必须释放显示层");
}

#[test]
fn reversing_a_transition_continues_from_the_current_frame() {
    let ui = fixture();
    ui.set_shown(true);
    assert_eq!(ui.get_progress(), 0.0);
    advance(70);
    let entering = ui.get_progress();
    assert!(entering > 0.0 && entering < 1.0);
    ui.set_shown(false);
    assert!(
        (ui.get_progress() - entering).abs() < 0.001,
        "反向关闭不应跳帧"
    );
    advance(40);
    let exiting = ui.get_progress();
    assert!(exiting > 0.0 && exiting < entering);
    ui.set_shown(true);
    assert!(
        (ui.get_progress() - exiting).abs() < 0.001,
        "退出中重开不应跳帧"
    );
    advance(200);
    assert_eq!(ui.get_progress(), 1.0);
    assert!(ui.get_present());
}

#[test]
fn closing_or_noninteractive_layer_does_not_dispatch_clicks_to_content() {
    let ui = fixture();
    ui.set_shown(true);
    assert_eq!(ui.get_progress(), 0.0);
    advance(200);
    click(&ui);
    assert_eq!(ui.get_clicks(), 1, "先证明输入确实命中测试内容");
    type_key(&ui);
    assert_eq!(ui.get_keys(), 1, "先证明内容确实获得键盘焦点");

    ui.set_interactive(false);
    click(&ui);
    type_key(&ui);
    assert_eq!(ui.get_clicks(), 1, "模态背景不得继续接收输入");
    assert_eq!(ui.get_keys(), 1, "模态背景中残留焦点不得处理键盘输入");
    ui.set_interactive(true);
    click(&ui);
    assert_eq!(ui.get_clicks(), 2);

    ui.set_shown(false);
    assert!(ui.get_present(), "验证发生在退出内容仍挂载时");
    click(&ui);
    type_key(&ui);
    assert_eq!(ui.get_clicks(), 2, "退出立即阻止内容输入");
    assert_eq!(ui.get_keys(), 1, "退场控件中残留焦点不得处理键盘输入");
    advance(200);
    click(&ui);
    assert_eq!(ui.get_clicks(), 2, "完全隐藏后不得响应点击");
}
