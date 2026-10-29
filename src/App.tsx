import { useState, useEffect, useRef } from "react";
import { flushSync } from "react-dom";
import type { TaskEditorHandle } from "./TaskFormDialog";
import { ProjectSidebar } from "./ProjectSidebar";
import { KanbanBoard } from "./KanbanBoard";
import { ToastContainer } from "./Toast";
import { Icon } from "./Icon";
import { TerminalPanel } from "./TerminalPanel";
import { useProjectSync } from "./useProjectSync";
import { TaskOverviewDialog } from "./TaskOverviewDialog";
import { TaskSummaryBoard } from "./TaskSummaryBoard";
import { useProjectTaskCounts } from "./useProjectTaskCounts";
import "./App.css";
import { tr, useLocale } from "./i18n";
import { WorkspaceAddDialog, machineLabel } from "./WorkspaceAddDialog";

export default function App() {
  useLocale();
  const { projects, navigation, hide, machines, machineErrors, machineBusy, gitStatuses, operations, dismissOperation, syncing, startingHerdr, startupSettled, error, refresh, session, reordering, move } = useProjectSync();
  const inProgressCounts = useProjectTaskCounts(projects);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [summaryOpen, setSummaryOpen] = useState(false);
  const [terminalMaximized, setTerminalMaximized] = useState(false);
  const [overviewOpen, setOverviewOpen] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const [recoveryId, setRecoveryId] = useState<string | null>(null);
  const editorRef = useRef<TaskEditorHandle>(null);

  useEffect(() => { setSelectedId((id) => projects.some((p) => p.id === id) ? id : projects[0]?.id ?? null); }, [projects]);

  const recoveryProject = navigation.recovery_projects.find(p => p.id === recoveryId);
  const selectedProject = recoveryProject ?? projects.find((p) => p.id === selectedId) ?? null;
  useEffect(() => { setTerminalMaximized(false); }, [selectedProject?.id]);
  const toggleTerminalMaximized = () => setTerminalMaximized(value => !value);

  async function selectProject(id: string, recovery = false) {
    // Close the overview before opening the editor's unsaved-changes dialog.
    if (overviewOpen) flushSync(() => setOverviewOpen(false));
    if ((summaryOpen || id !== selectedProject?.id) && await editorRef.current?.requestLeave() === false) return;
    setTerminalMaximized(false);
    setSummaryOpen(false);
    setRecoveryId(recovery ? id : null);
    if (!recovery) setSelectedId(id);
    setOverviewOpen(false);
  }

  async function openSummary() {
    if (summaryOpen) return;
    if (await editorRef.current?.requestLeave() === false) return;
    setTerminalMaximized(false);
    setSummaryOpen(true);
  }


  return (
    <div className={`app${terminalMaximized ? " terminal-maximized" : ""}`}>
      <ToastContainer />
      <ProjectSidebar
        machines={machines}
        machineErrors={machineErrors}
        machineBusy={machineBusy}
        inert={terminalMaximized}
        projects={projects}
        gitStatuses={gitStatuses}
        inProgressCounts={inProgressCounts}
        selectedId={summaryOpen ? null : selectedId}
        summarySelected={summaryOpen}
        onSummary={() => { void openSummary(); }}
        onSelect={id => { void selectProject(id); }}
        onHide={async id => { if (id !== editorRef.current?.projectId || await editorRef.current?.requestLeave() !== false) await hide(id); }}
        recoveryProjects={navigation.recovery_projects}
        onRecovery={id => { void selectProject(id, true); }}
        onChanged={refresh}
        onDismissOperation={dismissOperation}
        operations={operations}
        syncing={syncing}
        startingHerdr={startingHerdr}
        syncError={error}
        session={session}
        reordering={reordering}
        onMove={move}
        onOverview={() => setOverviewOpen(true)}
      />
      <main className="board-area">
        {!startupSettled && selectedProject && (selectedProject.machine_id ?? "local") === "local" ? <div className="board-workspace">
          <div className="board-main empty-state" inert={terminalMaximized}><p role="status">{tr("herdr.starting")}</p></div>
          <TerminalPanel task={null} maximized={terminalMaximized} onToggleMaximized={toggleTerminalMaximized} />
        </div> : <>
        {summaryOpen ? <TaskSummaryBoard projects={projects} machines={machines} editorRef={editorRef} terminalMaximized={terminalMaximized} onToggleTerminalMaximized={toggleTerminalMaximized} /> : selectedProject ? (
          <KanbanBoard key={selectedProject.id} project={selectedProject} editorRef={editorRef}
            machineName={machineLabel(selectedProject.machine_id ?? "local", machines.find(m => m.id === selectedProject.machine_id)?.name)}
            terminalMaximized={terminalMaximized} onToggleTerminalMaximized={toggleTerminalMaximized} />
        ) : (
          <div className="board-workspace">
            <div className="board-main empty-state" inert={terminalMaximized}>
              <span className="empty-state-icon"><Icon name="board" size={32} /></span>
              <span className="eyebrow">{tr("app.eyebrow")}</span>
              <h1 className="empty-state-title">{tr("app.empty_title")}</h1>
              <p className="empty-state-desc">{tr("app.empty_description")}</p>
              <p>{tr(navigation.configured ? "workspace.empty_selection" : "workspace.first_selection")}</p>
              <button className="btn-primary" onClick={() => setAddOpen(true)}>{tr("workspace.add")}</button>
            </div>
            <TerminalPanel task={null} maximized={terminalMaximized} onToggleMaximized={toggleTerminalMaximized} />
          </div>
        )}
        </>}
      </main>
      {overviewOpen && <TaskOverviewDialog projects={projects}
        onClose={() => setOverviewOpen(false)}
        onSelect={id => { void selectProject(id); }} />}
      {addOpen && <WorkspaceAddDialog machines={machines} onClose={() => setAddOpen(false)} onSaved={refresh} />}
    </div>
  );
}
