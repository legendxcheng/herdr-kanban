import { useState, useEffect } from "react";
import { ProjectSidebar } from "./ProjectSidebar";
import { KanbanBoard } from "./KanbanBoard";
import { ToastContainer } from "./Toast";
import { Icon } from "./Icon";
import { TerminalPanel } from "./TerminalPanel";
import { useProjectSync } from "./useProjectSync";
import { TaskOverviewDialog } from "./TaskOverviewDialog";
import "./App.css";
import { tr, useLocale } from "./i18n";

export default function App() {
  useLocale();
  const { projects, machines, machineErrors, machineBusy, gitStatuses, operations, syncing, error, refresh, session, reordering, move } = useProjectSync();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [terminalMaximized, setTerminalMaximized] = useState(false);
  const [overviewOpen, setOverviewOpen] = useState(false);

  useEffect(() => { setSelectedId((id) => projects.some((p) => p.id === id) ? id : projects[0]?.id ?? null); }, [projects]);

  const selectedProject = projects.find((p) => p.id === selectedId) ?? null;
  useEffect(() => { setTerminalMaximized(false); }, [selectedProject?.id]);
  const toggleTerminalMaximized = () => setTerminalMaximized(value => !value);

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
        selectedId={selectedId}
        onSelect={setSelectedId}
        onChanged={refresh}
        operations={operations}
        syncing={syncing}
        syncError={error}
        session={session}
        reordering={reordering}
        onMove={move}
        onOverview={() => setOverviewOpen(true)}
      />
      <main className="board-area">
        {selectedProject ? (
          <KanbanBoard key={selectedProject.id} project={selectedProject}
            machineName={machines.find(m => m.id === selectedProject.machine_id)?.name ?? tr("machine.local")}
            terminalMaximized={terminalMaximized} onToggleTerminalMaximized={toggleTerminalMaximized} />
        ) : (
          <div className="board-workspace">
            <div className="board-main empty-state" inert={terminalMaximized}>
              <span className="empty-state-icon"><Icon name="board" size={32} /></span>
              <span className="eyebrow">{tr("app.eyebrow")}</span>
              <h1 className="empty-state-title">{tr("app.empty_title")}</h1>
              <p className="empty-state-desc">{tr("app.empty_description")}</p>
            </div>
            <TerminalPanel task={null} maximized={terminalMaximized} onToggleMaximized={toggleTerminalMaximized} />
          </div>
        )}
      </main>
      {overviewOpen && <TaskOverviewDialog projects={projects}
        onClose={() => setOverviewOpen(false)}
        onSelect={id => { setSelectedId(id); setOverviewOpen(false); }} />}
    </div>
  );
}
