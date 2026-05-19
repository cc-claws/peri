use super::map_executor_event;
use crate::app::AgentEvent;
use peri_agent::agent::events::{AgentEvent as ExecutorEvent, TodoEntry, TodoStatus};
use peri_middlewares::tools::todo::TodoStatus as TuiTodoStatus;

#[test]
fn test_map_executor_event_todo_update() {
    let event = ExecutorEvent::TodoUpdate(vec![
        TodoEntry {
            content: "Fix the bug".into(),
            active_form: Some("Fixing the bug".into()),
            status: TodoStatus::InProgress,
        },
        TodoEntry {
            content: "Write tests".into(),
            active_form: None,
            status: TodoStatus::Pending,
        },
    ]);

    let result = map_executor_event(event, "/tmp");
    assert!(result.is_some(), "TodoUpdate must map to Some");

    match result.unwrap() {
        AgentEvent::TodoUpdate(todos) => {
            assert_eq!(todos.len(), 2);
            assert_eq!(todos[0].content, "Fix the bug");
            assert_eq!(todos[0].active_form, Some("Fixing the bug".into()));
            assert_eq!(todos[0].status, TuiTodoStatus::InProgress);
            assert_eq!(todos[1].content, "Write tests");
            assert_eq!(todos[1].active_form, None);
            assert_eq!(todos[1].status, TuiTodoStatus::Pending);
        }
        _ => panic!("Expected TodoUpdate, got a different variant"),
    }
}
