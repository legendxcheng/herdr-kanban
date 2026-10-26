import { TaskBoard, type TaskBoardProps } from "./KanbanBoard";

export function TaskSummaryBoard(props: Omit<TaskBoardProps, "project">) {
  return <TaskBoard {...props} />;
}
