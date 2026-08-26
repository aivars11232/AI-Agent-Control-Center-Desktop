import { useEffect, useRef, useState } from "react";
import type { Agent, AppPreferences, ApprovalRequest, VoiceState } from "../../applicationState";
import { persistenceErrorMessage } from "../../persistence";
import { interpretVoiceCommand } from "../../voiceCommand";
import { findActiveTemplateAgent } from "../../agentRegistry";
import { desktopClient, isDesktopRuntime } from "../../services/desktopClient";
import type { BackendActionIntent, DesktopControlStatus, VoiceRuntimeStatus } from "../../services/desktopClient";
import { errorMessage } from "../../domain/errors";
import type { TaskOrchestrationMutation } from "../contracts";
import { markApprovalConsumed, prepareBackendAuthorization, type AuthorizationReadiness } from "../../services/authorization";

type VoiceUiState = "VOICE OFF" | "PASSIVE" | "LISTENING" | "PROCESSING" | "EXECUTING" | "SUCCESS" | "ERROR";

type SpeechRecognitionLike = {
  lang: string;
  continuous: boolean;
  interimResults: boolean;
  start: () => void;
  onresult: ((event: {
    results: ArrayLike<ArrayLike<{ transcript: string }>>;
  }) => void) | null;
  onerror: ((event: { error: string }) => void) | null;
  onend: (() => void) | null;
};

type SpeechRecognitionConstructor = new () => SpeechRecognitionLike;

export function VoiceControlPage({
  agents,
  setAgents,
  onTaskMutation,
  setApprovalRequests,
  preferences,
  setPreferences,
  visible = true,
}: {
  agents: Agent[];
  setAgents: React.Dispatch<React.SetStateAction<Agent[]>>;
  onTaskMutation: TaskOrchestrationMutation;
  setApprovalRequests: React.Dispatch<
    React.SetStateAction<ApprovalRequest[]>
  >;
  preferences: AppPreferences;
  setPreferences: React.Dispatch<React.SetStateAction<AppPreferences>>;
  visible?: boolean;
}) {
  const [command, setCommand] = useState("");
  const [message, setMessage] = useState("");
  const [isListening, setIsListening] = useState(false);
  const [pendingApplication, setPendingApplication] = useState<string | null>(null);
  const [voiceRuntime, setVoiceRuntime] = useState<VoiceRuntimeStatus | null>(null);
  const [voiceState, setVoiceState] = useState<VoiceState>(
    preferences.voiceControlMasterEnabled ? preferences.voiceState : "VOICE_OFF",
  );
  const [voiceUiState, setVoiceUiState] = useState<VoiceUiState>(
    preferences.voiceControlMasterEnabled ? "PASSIVE" : "VOICE OFF",
  );
  const [desktopControl, setDesktopControl] = useState<DesktopControlStatus | null>(null);
  const submitCommandRef = useRef<(value: string) => void>(() => {});
  const desktopRestoreAttempted = useRef(false);
  const pcAgent = findActiveTemplateAgent(agents, "pc-control");
  const codingAgent = findActiveTemplateAgent(agents, "coding");
  const appAliases: Record<string, { key: string; label: string }> = {
    firefox: { key: "firefox", label: "Firefox" },
    dolphin: { key: "dolphin", label: "Dolphin" },
    "system settings": { key: "system-settings", label: "System Settings" },
    settings: { key: "system-settings", label: "System Settings" },
    terminal: { key: "terminal", label: "Terminal" },
    code: { key: "code", label: "Visual Studio Code" },
    "visual studio code": { key: "code", label: "Visual Studio Code" },
  };
  const pointerActions: Record<string, { key: string; label: string }> = {
    "move left": { key: "move-left", label: "move left" },
    "move right": { key: "move-right", label: "move right" },
    "move up": { key: "move-up", label: "move up" },
    "move down": { key: "move-down", label: "move down" },
    click: { key: "click", label: "click" },
    "double click": { key: "double-click", label: "double click" },
    "scroll up": { key: "scroll-up", label: "scroll up" },
    "scroll down": { key: "scroll-down", label: "scroll down" },
  };
  const desktopActionLabels: Record<string, string> = {
    "open-launcher": "open the application launcher",
    "volume-up": "increase volume",
    "volume-down": "decrease volume",
    "toggle-mute": "toggle mute",
    "minimize-window": "minimize the focused window",
    "maximize-window": "maximize the focused window",
    "restore-window": "restore the focused window",
    "next-window": "switch to the next window",
    "previous-window": "switch to the previous window",
    "snap-left": "snap the window left",
    "snap-right": "snap the window right",
    left: "move left",
    right: "move right",
    up: "move up",
    down: "move down",
    home: "go to the start",
    end: "go to the end",
    "page-up": "page up",
    "page-down": "page down",
    tab: "press Tab",
    "shift-tab": "press Shift+Tab",
    enter: "press Enter",
    escape: "press Escape",
    backspace: "press Backspace",
    delete: "press Delete",
    "select-all": "select all",
    copy: "copy",
    cut: "cut",
    paste: "paste",
    undo: "undo",
    redo: "redo",
  };
  const phraseList = (value: string) => value.split(",").map((phrase) => phrase.trim().toLowerCase()).filter(Boolean);
  const openPhrases = phraseList(preferences.voiceOpenPhrases);
  const closePhrases = phraseList(preferences.voiceClosePhrases);

  async function authorizeVoiceAction(
    intent: BackendActionIntent,
  ): Promise<AuthorizationReadiness | null> {
    try {
      const authorization = await prepareBackendAuthorization(
        intent,
        setApprovalRequests,
      );
      if (!authorization.ready) {
        setMessage(
          "This action is waiting for trusted backend authorization. Open Approvals to approve or deny it.",
        );
        setVoiceUiState("PROCESSING");
        return null;
      }
      return authorization;
    } catch (error) {
      setMessage(errorMessage(error));
      setVoiceUiState("ERROR");
      return null;
    }
  }

  async function createCodingTask(request: string) {
    if (!codingAgent) {
      setMessage("Lucy cannot route work because the Coding Agent is missing.");
      return;
    }
    if (!preferences.activeWorkspaceId) {
      setMessage("Select an active workspace before Lucy creates a task.");
      return;
    }
    try {
      await onTaskMutation("create_routed_task", {
        taskOwnerAgentId: codingAgent.id,
        title: request,
        category: "Development",
        priority: preferences.defaultTaskPriority,
        workspaceId: preferences.activeWorkspaceId,
        routingMode: "selected",
        preferredAgentId: codingAgent.id,
        selectedAgentId: codingAgent.id,
      });
      setMessage(
        `Lucy submitted a backend-owned coding task for ${codingAgent.name}.`,
      );
    } catch (error) {
      setMessage(persistenceErrorMessage(error));
      setVoiceUiState("ERROR");
    }
  }

  async function launchApplication(application: { key: string; label: string }) {
    if (!pcAgent || pcAgent.capabilities.system === "none") {
      setMessage("PC Control needs at least Minor system permission before it can open an application.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    if (pcAgent.approvals.system === "ask" && pcAgent.capabilities.system !== "full" && pendingApplication !== application.key) {
      setPendingApplication(application.key);
      setMessage(`Confirm opening ${application.label}. This is a one-time minor system action.`);
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Opening desktop applications is available in the installed Tauri app, not the browser preview.");
      return;
    }

    try {
      const authorization = await authorizeVoiceAction({
        kind: "launchAllowedApplication",
        agentId: pcAgent.id,
        application: application.key,
      });
      if (!authorization) return;
      await desktopClient.launchAllowedApplication(
        pcAgent.id,
        application.key,
      );
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setPendingApplication(null);
      setMessage(`Opened ${application.label}.`);
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  async function closeApplication(application: { key: string; label: string }) {
    if (!pcAgent || pcAgent.capabilities.system === "none") {
      setMessage("PC Control needs at least Minor system permission before it can close an application.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    const approvalKey = `close:${application.key}`;
    if (pcAgent.approvals.system === "ask" && pcAgent.capabilities.system !== "full" && pendingApplication !== approvalKey) {
      setPendingApplication(approvalKey);
      setMessage(`Confirm closing ${application.label}. This may ask the application to save unsaved work.`);
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Closing desktop applications is available in the installed Tauri app, not the browser preview.");
      return;
    }

    try {
      const authorization = await authorizeVoiceAction({
        kind: "closeAllowedApplication",
        agentId: pcAgent.id,
        application: application.key,
      });
      if (!authorization) return;
      await desktopClient.closeAllowedApplication(
        pcAgent.id,
        application.key,
      );
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setPendingApplication(null);
      setMessage(`Requested that ${application.label} close.`);
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  async function sendPointerAction(action: { key: string; label: string }) {
    if (!pcAgent || pcAgent.capabilities.system !== "full") {
      setMessage("Voice pointer control requires Full system permission for PC Control.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    const approvalKey = `pointer:${action.key}`;
    if (pcAgent.approvals.system === "ask" && pcAgent.capabilities.system !== "full" && pendingApplication !== approvalKey) {
      setPendingApplication(approvalKey);
      setMessage(`Confirm voice pointer action: ${action.label}.`);
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Voice pointer control is available in the installed Tauri app, not the browser preview.");
      return;
    }

    try {
      const authorization = await authorizeVoiceAction({
        kind: "desktopPointer",
        agentId: pcAgent.id,
        action: action.key,
      });
      if (!authorization) return;
      await desktopClient.sendDesktopPointerAction(pcAgent.id, action.key);
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setPendingApplication(null);
      setMessage(`Voice pointer: ${action.label}.`);
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  async function sendDesktopKeyboardAction(action: string) {
    if (!pcAgent || pcAgent.capabilities.system !== "full") {
      setMessage("Voice keyboard and volume controls require Full system permission for PC Control.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Voice keyboard controls are available in the installed Tauri app, not the browser preview.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "desktopKeyboard",
        agentId: pcAgent.id,
        action,
      });
      if (!authorization) return;
      await desktopClient.sendDesktopKeyboardAction(pcAgent.id, action);
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setVoiceUiState("SUCCESS");
      setMessage(`Requested: ${desktopActionLabels[action] ?? action}.`);
    } catch (error) {
      setVoiceUiState("ERROR");
      setMessage(errorMessage(error));
    }
  }

  async function controlNamedDesktopWindow(application: string, action: string) {
    if (!pcAgent || pcAgent.capabilities.system !== "full") {
      setMessage("Named application window controls require Full system permission for PC Control.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Named application window controls are available in the installed Tauri app, not the browser preview.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "desktopWindow",
        agentId: pcAgent.id,
        application,
        action,
      });
      if (!authorization) return;
      await desktopClient.controlNamedDesktopWindow(
        pcAgent.id,
        application,
        action,
      );
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setVoiceUiState("SUCCESS");
      setMessage(`Requested ${action} for the existing ${application} window.`);
    } catch (error) {
      setVoiceUiState("ERROR");
      setMessage(errorMessage(error));
    }
  }

  async function typeDesktopText(text: string) {
    if (!pcAgent || pcAgent.capabilities.system !== "full") {
      setMessage("Voice typing requires Full system permission for PC Control.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Voice typing is available in the installed Tauri app, not the browser preview.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "typeDesktopText",
        agentId: pcAgent.id,
        text,
      });
      if (!authorization) return;
      await desktopClient.typeDesktopText(pcAgent.id, text);
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setVoiceUiState("SUCCESS");
      setMessage("Typed dictated text into the focused application.");
    } catch (error) {
      setVoiceUiState("ERROR");
      setMessage(errorMessage(error));
    }
  }

  async function launchDesktopApplication(application: string) {
    if (!pcAgent || pcAgent.capabilities.system === "none") {
      setMessage("PC Control needs at least Minor system permission before it can open an application.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Opening desktop applications is available in the installed Tauri app, not the browser preview.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "launchDesktopApplication",
        agentId: pcAgent.id,
        application,
      });
      if (!authorization) return;
      await desktopClient.launchDesktopApplication(pcAgent.id, application);
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setVoiceUiState("SUCCESS");
      setMessage(`Opened ${application}.`);
    } catch (error) {
      setVoiceUiState("ERROR");
      setMessage(errorMessage(error));
    }
  }

  async function openStandardFolder(folder: string) {
    if (!pcAgent) {
      setMessage("PC Control Agent is unavailable.");
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Opening folders is available in the installed Tauri app, not the browser preview.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "openStandardFolder",
        agentId: pcAgent.id,
        folder,
      });
      if (!authorization) return;
      await desktopClient.openStandardFolder(pcAgent.id, folder);
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setVoiceUiState("SUCCESS");
      setMessage(`Opened ${folder}.`);
    } catch (error) {
      setVoiceUiState("ERROR");
      setMessage(errorMessage(error));
    }
  }

  async function closeActiveDesktopApplication() {
    if (!pcAgent || pcAgent.capabilities.system !== "full") {
      setMessage("Closing the active application by voice requires Full system permission for PC Control.");
      return;
    }
    if (pcAgent.approvals.system === "deny") {
      setMessage("PC Control system actions are denied by its approval policy.");
      return;
    }
    if (!isDesktopRuntime()) {
      setMessage("Closing the active desktop application is available in the installed Tauri app, not the browser preview.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "closeActiveApplication",
        agentId: pcAgent.id,
      });
      if (!authorization) return;
      await desktopClient.closeActiveDesktopApplication(pcAgent.id);
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setMessage("Requested that the active application close.");
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  function submitCommand(value = command) {
    const understood = interpretVoiceCommand(value, {
      openPhrases,
      closePhrases,
      replacements: preferences.voiceCommandReplacements,
    });
    setVoiceUiState("PROCESSING");
    setMessage(`Processing: ${understood.transcript}`);
    if (understood.intent === "coding_request") {
      void createCodingTask(understood.entity);
      return;
    }
    if (understood.intent === "open_folder") {
      setVoiceUiState("EXECUTING");
      void openStandardFolder(understood.entity);
      return;
    }
    if (understood.intent === "open_application") {
      setVoiceUiState("EXECUTING");
      const application = appAliases[understood.entity];
      void (application ? launchApplication(application) : launchDesktopApplication(understood.entity));
      return;
    }
    if (understood.intent === "close_application") {
      setVoiceUiState("EXECUTING");
      const application = appAliases[understood.entity];
      void (application && application.key !== "terminal" ? closeApplication(application) : closeActiveDesktopApplication());
      return;
    }
    if (understood.intent === "pointer_action") {
      setVoiceUiState("EXECUTING");
      const action = Object.values(pointerActions).find((candidate) => candidate.key === understood.entity);
      if (action) void sendPointerAction(action);
      return;
    }
    if (understood.intent === "desktop_action") {
      setVoiceUiState("EXECUTING");
      void sendDesktopKeyboardAction(understood.entity);
      return;
    }
    if (understood.intent === "application_window_action") {
      setVoiceUiState("EXECUTING");
      void controlNamedDesktopWindow(understood.entity, understood.action ?? "restore");
      return;
    }
    if (understood.intent === "text_input") {
      setVoiceUiState("EXECUTING");
      void typeDesktopText(understood.entity);
      return;
    }
    setVoiceUiState("ERROR");
    setMessage("Lucy could not map that phrase to a supported, safe command.");
  }

  submitCommandRef.current = submitCommand;

  useEffect(() => {
    if (!isDesktopRuntime()) return;
    let active = true;
    void desktopClient.voiceRuntimeStatus()
      .then((status) => {
        if (active) setVoiceRuntime(status);
      })
      .catch((error) => {
        if (active) setMessage(errorMessage(error));
      });
    void desktopClient.desktopControlStatus()
      .then((status) => {
        if (active) setDesktopControl(status);
      })
      .catch((error) => {
        if (active) setMessage(errorMessage(error));
      });
    const unlistenStatus = desktopClient.onVoiceRuntimeStatus((status) => {
      setVoiceRuntime(status);
      setIsListening(status.listening);
      setMessage(status.message);
    });
    const unlistenTranscript = desktopClient.onVoiceTranscript((event) => {
      const { kind, transcript } = event;
      if (kind === "activated") {
        setVoiceState("VOICE_ACTIVE");
        setVoiceUiState("LISTENING");
        setPreferences((current) => ({ ...current, voiceState: "VOICE_ACTIVE" }));
        setMessage(`Lucy is active. Say ${preferences.voiceDeactivatePhrase} to return to wake-only mode.`);
        return;
      }
      if (kind === "deactivated") {
        setVoiceState("VOICE_PASSIVE");
        setVoiceUiState("PASSIVE");
        setPreferences((current) => ({ ...current, voiceState: "VOICE_PASSIVE" }));
        setMessage(`Lucy command mode is off. Say ${preferences.voiceWakePhrase} when you want to control your PC.`);
        return;
      }
      if (kind === "off_requested") {
        setVoiceState("VOICE_OFF");
        setVoiceUiState("VOICE OFF");
        setIsListening(false);
        setPreferences((current) => ({
          ...current,
          voiceControlMasterEnabled: false,
          voiceState: "VOICE_OFF",
        }));
        void desktopClient.stopVoiceListener()
          .then(() => setMessage("Lucy voice control is off. Re-enable it from Voice Control to start listening again."))
          .catch((error) => setMessage(errorMessage(error)));
        return;
      }
      if (kind === "listening") {
        setVoiceUiState("LISTENING");
        setMessage("Lucy is listening for your command.");
        return;
      }
      if (kind === "ready") {
        setIsListening(true);
        setVoiceState("VOICE_PASSIVE");
        setVoiceUiState("PASSIVE");
        setPreferences((current) => ({ ...current, voiceState: "VOICE_PASSIVE" }));
        setMessage(`Lucy wake listener is active. Say ${preferences.voiceWakePhrase} to begin giving commands.`);
        return;
      }
      if (kind === "error") {
        setIsListening(false);
        setVoiceUiState("ERROR");
        setMessage(transcript);
        return;
      }
      if (kind === "heard") {
        setVoiceUiState("LISTENING");
        setMessage(`Listening: ${transcript}`);
        return;
      }
      if (!transcript.trim()) return;
      setCommand(transcript);
      setMessage(`Executing: ${transcript}`);
      submitCommandRef.current(transcript);
    });
    return () => {
      active = false;
      void unlistenStatus.then((unlisten) => unlisten());
      void unlistenTranscript.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    if (!isDesktopRuntime() || !preferences.backgroundVoiceEnabled || !preferences.voiceControlMasterEnabled) return;
    if (pcAgent) {
      void startListening();
    }
  }, [preferences.backgroundVoiceEnabled, preferences.voiceControlMasterEnabled, preferences.voiceWakePhrase]);

  function toggleBackgroundVoice() {
    const nextEnabled = !preferences.backgroundVoiceEnabled;
    setPreferences((current) => ({ ...current, backgroundVoiceEnabled: nextEnabled }));
    if (!nextEnabled) {
      void desktopClient.stopVoiceListener()
        .then(() => {
          setIsListening(false);
          setVoiceState("VOICE_OFF");
          setVoiceUiState("VOICE OFF");
          setPreferences((current) => ({ ...current, voiceState: "VOICE_OFF" }));
          setMessage("Background voice mode is off. Manual in-app listening is still available.");
        })
        .catch((error) => setMessage(errorMessage(error)));
    }
  }

  function toggleLucyMaster() {
    const nextEnabled = !preferences.voiceControlMasterEnabled;
    setPreferences((current) => ({ ...current, voiceControlMasterEnabled: nextEnabled }));
    if (!nextEnabled) {
      void desktopClient.stopVoiceListener()
        .then(() => {
          setIsListening(false);
          setVoiceState("VOICE_OFF");
          setVoiceUiState("VOICE OFF");
          setPreferences((current) => ({ ...current, voiceState: "VOICE_OFF" }));
          setMessage("Lucy is completely disabled. No microphone audio is being captured.");
        })
        .catch((error) => setMessage(errorMessage(error)));
      } else {
        setVoiceState("VOICE_PASSIVE");
        setVoiceUiState("PASSIVE");
        setPreferences((current) => ({ ...current, voiceState: "VOICE_PASSIVE" }));
    }
  }

  function updateVoicePreference(key: "voiceWakePhrase" | "voiceDeactivatePhrase" | "voiceOpenPhrases" | "voiceClosePhrases" | "voiceCommandReplacements", value: string) {
    setPreferences((current) => ({ ...current, [key]: value.toLowerCase() }));
  }

  async function installOfflineVoice() {
    if (!pcAgent) {
      setMessage("PC Control Agent is unavailable.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "installVoiceRuntime",
        agentId: pcAgent.id,
      });
      if (!authorization) return;
      await desktopClient.installVoiceRuntime(pcAgent.id);
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setMessage("Downloading the local speech model. Keep the app open until installation finishes.");
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  async function installHighAccuracyVoice() {
    if (!pcAgent) {
      setMessage("PC Control Agent is unavailable.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "installHighAccuracyVoiceRuntime",
        agentId: pcAgent.id,
      });
      if (!authorization) return;
      await desktopClient.installHighAccuracyVoiceRuntime(pcAgent.id);
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setMessage("Building the high-accuracy speech engine and downloading its local model. This can take several minutes.");
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  async function enableDesktopControl() {
    if (!pcAgent || pcAgent.capabilities.system !== "full") {
      setMessage("Set PC Control to Full system permission before enabling desktop input.");
      return;
    }
    try {
      const authorization = await authorizeVoiceAction({
        kind: "enableDesktopControl",
        agentId: pcAgent.id,
      });
      if (!authorization) return;
      const status = await desktopClient.enableDesktopControl(pcAgent.id);
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setDesktopControl(status);
      setMessage(status.message);
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  useEffect(() => {
    if (!isDesktopRuntime() || pcAgent?.capabilities.system !== "full") {
      desktopRestoreAttempted.current = false;
      return;
    }
    if (desktopControl?.enabled || desktopRestoreAttempted.current) return;
    desktopRestoreAttempted.current = true;
    void enableDesktopControl();
  }, [desktopControl?.enabled, pcAgent?.capabilities.system]);

  function activateFullSystemPermission() {
    if (!pcAgent) {
      setMessage("PC Control Agent is unavailable.");
      return;
    }
    if (pcAgent.capabilities.system === "full") {
      void enableDesktopControl();
      return;
    }
    setAgents((currentAgents) =>
      currentAgents.map((agent) =>
        agent.id === pcAgent.id
          ? {
              ...agent,
              capabilities: { ...agent.capabilities, system: "full" },
              approvals: { ...agent.approvals, system: "allow" },
              activity: [
                {
                  id: Date.now(),
                  message: "Full system permission enabled from Voice Control.",
                  createdAt: new Date().toISOString(),
                },
                ...agent.activity,
              ],
            }
          : agent,
      ),
    );
    setMessage("Full system permission is active. Requesting KDE desktop input permission...");
    void enableDesktopControl();
  }

  function startListening() {
    if (isDesktopRuntime()) {
      if (!pcAgent) {
        setMessage("PC Control Agent is unavailable.");
        return;
      }
      void authorizeVoiceAction({
        kind: "startVoiceListener",
        agentId: pcAgent.id,
      }).then((authorization) => {
        if (!authorization) return;
        void desktopClient.startVoiceListener(pcAgent.id)
          .then(() => {
            markApprovalConsumed(setApprovalRequests, authorization.approval);
            setVoiceUiState("PROCESSING");
            setMessage("Starting Lucy's microphone listener...");
          })
          .catch((error) => {
            setIsListening(false);
            setVoiceUiState("ERROR");
            setMessage(errorMessage(error));
          });
      });
      return;
    }
    const speechWindow = window as typeof window & {
      SpeechRecognition?: SpeechRecognitionConstructor;
      webkitSpeechRecognition?: SpeechRecognitionConstructor;
    };
    const Recognition = speechWindow.SpeechRecognition ?? speechWindow.webkitSpeechRecognition;
    if (!Recognition) {
      setMessage("Speech recognition is not available in this webview. Type a command instead.");
      return;
    }
    const recognition = new Recognition();
    recognition.lang = "en-US";
    recognition.continuous = false;
    recognition.interimResults = false;
    recognition.onresult = (event) => {
      const transcript = event.results[0]?.[0]?.transcript ?? "";
      setCommand(transcript);
      submitCommand(transcript);
    };
    recognition.onerror = () => setMessage("Voice recognition could not understand that command. Try again or type it.");
    recognition.onend = () => setIsListening(false);
    setIsListening(true);
    recognition.start();
  }

  function stopListening() {
    if (isDesktopRuntime()) {
      void desktopClient.stopVoiceListener()
        .then(() => setIsListening(false))
        .catch((error) => setMessage(errorMessage(error)));
      return;
    }
    setIsListening(false);
  }

  const permissionLabel = pcAgent?.capabilities.system === "full"
    ? "Full system permission"
    : pcAgent?.capabilities.system === "power"
      ? "Elevated system permission"
      : pcAgent?.capabilities.system === "notifications"
        ? "Minor system permission"
        : "No system permission";

  return (
    <div hidden={!visible}>
      <header className="topbar">
        <div>
          <span className="eyebrow">VOICE AND TEXT COMMANDS</span>
          <h1>Voice Control</h1>
          <p className="page-message">Say an approved app name directly, or use Lucy to create a coding task.</p>
        </div>
      </header>

      <section className="summary-grid">
        <article className="summary-card"><span>PC Control</span><strong>{pcAgent?.status ?? "Unavailable"}</strong><small>{permissionLabel}{pcAgent?.capabilities.system === "full" ? " active" : ""}</small></article>
        <article className="summary-card"><span>Approval policy</span><strong>{pcAgent?.approvals.system ?? "deny"}</strong><small>System action authorization</small></article>
        <article className="summary-card"><span>Lucy</span><strong>{codingAgent ? "Ready" : "Unavailable"}</strong><small>Routes coding requests to {codingAgent?.name ?? "no agent"}</small></article>
        <article className="summary-card"><span>Voice state</span><strong>{voiceState.replace("VOICE_", "")}</strong><small>{voiceState === "VOICE_OFF" ? "Microphone disabled" : voiceState === "VOICE_PASSIVE" ? `Waiting for ${preferences.voiceWakePhrase}` : "Accepting commands"}</small></article>
        <article className="summary-card"><span>Command status</span><strong>{voiceUiState}</strong><small>Voice command lifecycle</small></article>
      </section>

      {isDesktopRuntime() && (
        <section className="settings-note">
          Closing the window keeps the control center running in the system tray. Offline voice uses your microphone through PipeWire and processes speech locally; it does not use the webview speech API.
        </section>
      )}

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">COMMAND CENTER</span>
            <h2>Speak or type a request</h2>
          </div>
          <div className="button-row">
            {isDesktopRuntime() && !voiceRuntime?.installed && <button className="secondary-button" onClick={() => void installOfflineVoice()}>Install offline voice engine</button>}
            {isDesktopRuntime() && voiceRuntime?.installed && !voiceRuntime.highAccuracyAvailable && <button className="secondary-button" onClick={() => void installHighAccuracyVoice()}>Install high-accuracy voice</button>}
            {isDesktopRuntime() && <button className="secondary-button" onClick={toggleBackgroundVoice}>{preferences.backgroundVoiceEnabled ? "Disable wake listener" : "Enable wake listener"}</button>}
            {isDesktopRuntime() && <button className="secondary-button" onClick={toggleLucyMaster}>{preferences.voiceControlMasterEnabled ? "Disable Lucy completely" : "Enable Lucy"}</button>}
            <button className="primary-button microphone-button" onClick={isListening ? stopListening : startListening} disabled={!preferences.voiceControlMasterEnabled}>
              <span className={`microphone-indicator ${voiceState === "VOICE_ACTIVE" ? "is-active" : isListening ? "is-passive" : "is-off"}`} aria-hidden="true" />
              {isListening ? "Stop listening" : "Listen"}
            </button>
          </div>
        </div>
        <div className="model-composer">
          <label className="form-field">
            <span>Command</span>
            <input value={command} onChange={(event) => setCommand(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") submitCommand(); }} placeholder="Firefox, open Firefox, or Lucy fix the build error" />
          </label>
          <button className="primary-button" onClick={() => submitCommand()}>Run command</button>
        </div>
        {message && <div className="runtime-message">{message}</div>}
        {isDesktopRuntime() && desktopControl && <p className="form-hint">{desktopControl.message}</p>}
        {isDesktopRuntime() && !desktopControl?.enabled && <button className="secondary-button" onClick={() => void enableDesktopControl()}>Enable KDE desktop input</button>}
        {isDesktopRuntime() && voiceRuntime && <p className="form-hint">{voiceRuntime.message}{voiceRuntime.highAccuracyAvailable ? " Whisper base.en transcribes commands after Lucy wakes." : " Install high-accuracy voice for a broader command vocabulary."}{preferences.backgroundVoiceEnabled && preferences.voiceControlMasterEnabled ? " Lucy waits for its wake phrase while this app is in the tray." : ""}</p>}
      </section>

      <section className="panel">
        <div className="panel-heading"><div><span className="eyebrow">LUCY PHRASES</span><h2>Wake and command vocabulary</h2></div></div>
        <div className="form-grid">
          <label className="form-field"><span>Wake phrase</span><input value={preferences.voiceWakePhrase} onChange={(event) => updateVoicePreference("voiceWakePhrase", event.target.value)} placeholder="lucy activate" /></label>
          <label className="form-field"><span>Deactivate phrase</span><input value={preferences.voiceDeactivatePhrase} onChange={(event) => updateVoicePreference("voiceDeactivatePhrase", event.target.value)} placeholder="lucy deactivate" /></label>
          <label className="form-field"><span>Open phrases</span><input value={preferences.voiceOpenPhrases} onChange={(event) => updateVoicePreference("voiceOpenPhrases", event.target.value)} placeholder="open, launch, start" /></label>
          <label className="form-field"><span>Close phrases</span><input value={preferences.voiceClosePhrases} onChange={(event) => updateVoicePreference("voiceClosePhrases", event.target.value)} placeholder="close, quit, exit" /></label>
          <label className="form-field"><span>Recognition replacements</span><textarea value={preferences.voiceCommandReplacements} onChange={(event) => updateVoicePreference("voiceCommandReplacements", event.target.value)} placeholder="fire fox = firefox" /></label>
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading"><div><span className="eyebrow">PERMISSION LADDER</span><h2>System-control boundaries</h2></div></div>
        <div className="agent-list">
          <article className="agent-card"><div><h3>None</h3><p>No desktop actions are accepted.</p></div></article>
          <article className="agent-card"><div><h3>Minor</h3><p>Allowlisted application launches only. Firefox and “open Firefox” are equivalent.</p></div></article>
          <article className="agent-card"><div><h3>Elevated</h3><p>Reserved for future confirmed power actions. It does not enable arbitrary commands.</p></div></article>
          <article className="agent-card">
            <div>
              <h3>Full {pcAgent?.capabilities.system === "full" ? "- Active" : ""}</h3>
              <p>Enables KDE desktop pointer and keyboard permission. Administrator commands remain blocked.</p>
              <button className="secondary-button" onClick={activateFullSystemPermission} disabled={!pcAgent}>
                {pcAgent?.capabilities.system === "full"
                  ? desktopControl?.enabled
                    ? "Full control active"
                    : "Restore KDE desktop input"
                  : "Enable full control"}
              </button>
            </div>
          </article>
        </div>
      </section>
    </div>
  );
}
