import { useEffect, useRef, useState } from "react";
import type {
  Agent,
  AppPreferences,
  ApprovalRequest,
  VoiceState,
} from "../../applicationState";
import { findActiveTemplateAgent } from "../../agentRegistry";
import { errorMessage } from "../../domain/errors";
import {
  desktopClient,
  isDesktopRuntime,
  type BackendActionIntent,
  type DesktopControlStatus,
  type SubmitVoiceIntentRequest,
  type SystemActionAuditRecord,
  type VoiceRuntimeStatus,
} from "../../services/desktopClient";
import {
  markApprovalConsumed,
  prepareBackendAuthorization,
  upsertApprovalRequest,
  type AuthorizationReadiness,
} from "../../services/authorization";
import { interpretVoiceCommand } from "../../voiceCommand";

type VoiceUiState =
  | "VOICE OFF"
  | "PASSIVE"
  | "LISTENING"
  | "PROCESSING"
  | "EXECUTING"
  | "SUCCESS"
  | "ERROR";

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

let fallbackRequestSequence = 0;

export function createVoiceRequestId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `voice:${crypto.randomUUID()}`;
  }
  fallbackRequestSequence += 1;
  return `voice:${Date.now()}:${fallbackRequestSequence}`;
}

function gatewayUiState(
  status: SystemActionAuditRecord["status"],
): VoiceUiState {
  if (status === "taskCreated" || status === "applied") return "SUCCESS";
  if (status === "approvalRequired") return "PROCESSING";
  if (status === "dispatched") return "EXECUTING";
  return "ERROR";
}

export function VoiceControlPage({
  agents,
  onGatewayMutation,
  setApprovalRequests,
  preferences,
  setPreferences,
  visible = true,
}: {
  agents: Agent[];
  onGatewayMutation: () => Promise<void>;
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
  const [voiceRuntime, setVoiceRuntime] =
    useState<VoiceRuntimeStatus | null>(null);
  const [voiceState, setVoiceState] = useState<VoiceState>(
    preferences.voiceControlMasterEnabled
      ? preferences.voiceState
      : "VOICE_OFF",
  );
  const [voiceUiState, setVoiceUiState] = useState<VoiceUiState>(
    preferences.voiceControlMasterEnabled ? "PASSIVE" : "VOICE OFF",
  );
  const [desktopControl, setDesktopControl] =
    useState<DesktopControlStatus | null>(null);
  const [pendingSubmission, setPendingSubmission] =
    useState<SubmitVoiceIntentRequest | null>(null);
  const [audits, setAudits] = useState<SystemActionAuditRecord[]>([]);
  const [gatewayBusy, setGatewayBusy] = useState(false);
  const submitCommandRef = useRef<(value: string) => void>(() => {});
  const pcAgent = findActiveTemplateAgent(agents, "pc-control");
  const codingAgent = findActiveTemplateAgent(agents, "coding");
  const phraseList = (value: string) =>
    value
      .split(",")
      .map((phrase) => phrase.trim().toLowerCase())
      .filter(Boolean);
  const openPhrases = phraseList(preferences.voiceOpenPhrases);
  const closePhrases = phraseList(preferences.voiceClosePhrases);

  async function authorizeVoiceLifecycle(
    intent: BackendActionIntent,
  ): Promise<AuthorizationReadiness | null> {
    try {
      const authorization = await prepareBackendAuthorization(
        intent,
        setApprovalRequests,
      );
      if (!authorization.ready) {
        setMessage(
          "This lifecycle action is waiting for backend authorization. Open Approvals to approve or deny it.",
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

  async function refreshAudits(reportError: boolean) {
    if (!isDesktopRuntime()) return;
    try {
      const page = await desktopClient.querySystemActionAudits(50);
      setAudits(page.records);
    } catch (error) {
      if (reportError) {
        setMessage(errorMessage(error));
        setVoiceUiState("ERROR");
      }
    }
  }

  async function submitGatewayRequest(request: SubmitVoiceIntentRequest) {
    if (!isDesktopRuntime()) {
      setMessage(
        "The authoritative command gateway is available in the installed Tauri app, not the browser preview.",
      );
      setVoiceUiState("ERROR");
      return;
    }
    setGatewayBusy(true);
    setVoiceUiState("EXECUTING");
    try {
      const result = await desktopClient.submitVoiceIntent(request);
      const approval = result.approval;
      if (approval) {
        setApprovalRequests((requests) =>
          upsertApprovalRequest(requests, approval),
        );
      }
      setAudits((records) => [
        result.audit,
        ...records.filter((record) => record.id !== result.audit.id),
      ]);
      setMessage(result.message);
      setVoiceUiState(gatewayUiState(result.status));
      if (result.status === "approvalRequired") {
        setPendingSubmission(request);
      } else {
        setPendingSubmission((current) =>
          current?.requestId === request.requestId ? null : current,
        );
      }
      if (
        result.status === "taskCreated" ||
        Boolean(result.approval?.consumedAt)
      ) {
        await onGatewayMutation();
      }
      await refreshAudits(false);
    } catch (error) {
      setMessage(errorMessage(error));
      setVoiceUiState("ERROR");
    } finally {
      setGatewayBusy(false);
    }
  }

  function submitCommand(value = command) {
    const understood = interpretVoiceCommand(value, {
      openPhrases,
      closePhrases,
      replacements: preferences.voiceCommandReplacements,
    });
    if (!understood.intent) {
      setVoiceUiState("ERROR");
      setMessage("Lucy could not map that phrase to a supported, safe command.");
      return;
    }
    setPendingSubmission(null);
    setVoiceUiState("PROCESSING");
    setMessage(`Processing locally: ${understood.transcript}`);
    void submitGatewayRequest({
      requestId: createVoiceRequestId(),
      intent: understood.intent,
    });
  }

  submitCommandRef.current = submitCommand;

  useEffect(() => {
    if (!isDesktopRuntime()) return;
    let active = true;
    void desktopClient
      .voiceRuntimeStatus()
      .then((status) => {
        if (active) setVoiceRuntime(status);
      })
      .catch((error) => {
        if (active) setMessage(errorMessage(error));
      });
    void desktopClient
      .desktopControlStatus()
      .then((status) => {
        if (active) setDesktopControl(status);
      })
      .catch((error) => {
        if (active) setMessage(errorMessage(error));
      });
    void desktopClient
      .querySystemActionAudits(50)
      .then((page) => {
        if (active) setAudits(page.records);
      })
      .catch((error) => {
        if (active) setMessage(errorMessage(error));
      });
    const unlistenStatus = desktopClient.onVoiceRuntimeStatus((status) => {
      setVoiceRuntime(status);
      setIsListening(status.listening);
      setMessage(status.message);
    });
    const unlistenDesktopControl = desktopClient.onDesktopControlStatus(
      (status) => {
        setDesktopControl(status);
        setMessage(status.message);
      },
    );
    const unlistenTranscript = desktopClient.onVoiceTranscript((event) => {
      const { kind, transcript } = event;
      if (kind === "activated") {
        setVoiceState("VOICE_ACTIVE");
        setVoiceUiState("LISTENING");
        setPreferences((current) => ({
          ...current,
          voiceState: "VOICE_ACTIVE",
        }));
        setMessage(
          `Lucy is active. Say ${preferences.voiceDeactivatePhrase} to return to wake-only mode.`,
        );
        return;
      }
      if (kind === "deactivated") {
        setVoiceState("VOICE_PASSIVE");
        setVoiceUiState("PASSIVE");
        setPreferences((current) => ({
          ...current,
          voiceState: "VOICE_PASSIVE",
        }));
        setMessage(
          `Lucy command mode is off. Say ${preferences.voiceWakePhrase} when you want to control your PC.`,
        );
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
        void desktopClient
          .stopVoiceListener()
          .then(() =>
            setMessage(
              "Lucy voice control is off. Re-enable it from Voice Control to start listening again.",
            ),
          )
          .catch((error) => setMessage(errorMessage(error)));
        return;
      }
      if (kind === "listening") {
        setVoiceUiState("LISTENING");
        setMessage("Lucy is listening for your command.");
        return;
      }
      if (kind === "transcribing") {
        setVoiceUiState("PROCESSING");
        setMessage("The optional local high-accuracy engine is transcribing.");
        return;
      }
      if (kind === "warning") {
        setMessage(transcript);
        return;
      }
      if (kind === "ready") {
        setIsListening(true);
        setVoiceState("VOICE_PASSIVE");
        setVoiceUiState("PASSIVE");
        setPreferences((current) => ({
          ...current,
          voiceState: "VOICE_PASSIVE",
        }));
        setMessage(
          `Lucy wake listener is active. Say ${preferences.voiceWakePhrase} to begin giving commands.`,
        );
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
      submitCommandRef.current(transcript);
    });
    return () => {
      active = false;
      void unlistenStatus.then((unlisten) => unlisten());
      void unlistenDesktopControl.then((unlisten) => unlisten());
      void unlistenTranscript.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    if (
      !isDesktopRuntime() ||
      !preferences.backgroundVoiceEnabled ||
      !preferences.voiceControlMasterEnabled
    ) {
      return;
    }
    if (pcAgent) {
      startListening();
    }
  }, [
    preferences.backgroundVoiceEnabled,
    preferences.voiceControlMasterEnabled,
    preferences.voiceWakePhrase,
  ]);

  function toggleBackgroundVoice() {
    const nextEnabled = !preferences.backgroundVoiceEnabled;
    setPreferences((current) => ({
      ...current,
      backgroundVoiceEnabled: nextEnabled,
    }));
    if (!nextEnabled) {
      void desktopClient
        .stopVoiceListener()
        .then(() => {
          setIsListening(false);
          setVoiceState("VOICE_OFF");
          setVoiceUiState("VOICE OFF");
          setPreferences((current) => ({
            ...current,
            voiceState: "VOICE_OFF",
          }));
          setMessage(
            "Background voice mode is off. Manual in-app listening is still available.",
          );
        })
        .catch((error) => setMessage(errorMessage(error)));
    }
  }

  function toggleLucyMaster() {
    const nextEnabled = !preferences.voiceControlMasterEnabled;
    setPreferences((current) => ({
      ...current,
      voiceControlMasterEnabled: nextEnabled,
    }));
    if (!nextEnabled) {
      void desktopClient
        .stopVoiceListener()
        .then(() => {
          setIsListening(false);
          setVoiceState("VOICE_OFF");
          setVoiceUiState("VOICE OFF");
          setPreferences((current) => ({
            ...current,
            voiceState: "VOICE_OFF",
          }));
          setMessage(
            "Lucy is completely disabled. No microphone audio is being captured.",
          );
        })
        .catch((error) => setMessage(errorMessage(error)));
    } else {
      setVoiceState("VOICE_PASSIVE");
      setVoiceUiState("PASSIVE");
      setPreferences((current) => ({
        ...current,
        voiceState: "VOICE_PASSIVE",
      }));
    }
  }

  function updateVoicePreference(
    key:
      | "voiceWakePhrase"
      | "voiceDeactivatePhrase"
      | "voiceOpenPhrases"
      | "voiceClosePhrases"
      | "voiceCommandReplacements",
    value: string,
  ) {
    setPreferences((current) => ({
      ...current,
      [key]: value.toLowerCase(),
    }));
  }

  async function installOfflineVoice() {
    if (!pcAgent) {
      setMessage("PC Control Agent is unavailable.");
      return;
    }
    try {
      const authorization = await authorizeVoiceLifecycle({
        kind: "installVoiceRuntime",
        agentId: pcAgent.id,
      });
      if (!authorization) return;
      const status = await desktopClient.installVoiceRuntime(pcAgent.id);
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setVoiceRuntime(status);
      setMessage(status.message);
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
      const authorization = await authorizeVoiceLifecycle({
        kind: "installHighAccuracyVoiceRuntime",
        agentId: pcAgent.id,
      });
      if (!authorization) return;
      const status = await desktopClient.installHighAccuracyVoiceRuntime(
        pcAgent.id,
      );
      markApprovalConsumed(setApprovalRequests, authorization.approval);
      setVoiceRuntime(status);
      setMessage(status.message);
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  async function cancelVoiceInstall() {
    if (!voiceRuntime?.operationId) return;
    try {
      const status = await desktopClient.cancelVoiceRuntimeInstall(
        voiceRuntime.operationId,
      );
      setVoiceRuntime(status);
      setMessage(status.message);
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  async function enableDesktopControl() {
    if (!pcAgent || pcAgent.capabilities.system !== "full") {
      setMessage(
        "Configure Full PC Control permission in Agents before requesting KDE desktop input.",
      );
      return;
    }
    try {
      const authorization = await authorizeVoiceLifecycle({
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

  async function disableDesktopControl() {
    try {
      const status = await desktopClient.disableDesktopControl();
      setDesktopControl(status);
      setMessage(status.message);
    } catch (error) {
      setMessage(errorMessage(error));
    }
  }

  function startListening() {
    if (isDesktopRuntime()) {
      if (!pcAgent) {
        setMessage("PC Control Agent is unavailable.");
        return;
      }
      void authorizeVoiceLifecycle({
        kind: "startVoiceListener",
        agentId: pcAgent.id,
      }).then((authorization) => {
        if (!authorization) return;
        void desktopClient
          .startVoiceListener(pcAgent.id)
          .then((status) => {
            markApprovalConsumed(
              setApprovalRequests,
              authorization.approval,
            );
            setVoiceRuntime(status);
            setVoiceUiState("PROCESSING");
            setMessage(status.message);
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
    const Recognition =
      speechWindow.SpeechRecognition ?? speechWindow.webkitSpeechRecognition;
    if (!Recognition) {
      setMessage(
        "Speech recognition is not available in this webview. Type a command instead.",
      );
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
    recognition.onerror = () =>
      setMessage(
        "Voice recognition could not understand that command. Try again or type it.",
      );
    recognition.onend = () => setIsListening(false);
    setIsListening(true);
    recognition.start();
  }

  function stopListening() {
    if (isDesktopRuntime()) {
      void desktopClient
        .stopVoiceListener()
        .then((status) => {
          setVoiceRuntime(status);
          setIsListening(false);
          setMessage(status.message);
        })
        .catch((error) => setMessage(errorMessage(error)));
      return;
    }
    setIsListening(false);
  }

  const permissionLabel =
    pcAgent?.capabilities.system === "full"
      ? "Full system permission"
      : pcAgent?.capabilities.system === "power"
        ? "Elevated system permission"
        : pcAgent?.capabilities.system === "notifications"
          ? "Minor system permission"
          : "No system permission";
  const voiceInstallActive =
    voiceRuntime?.installState === "installing" ||
    voiceRuntime?.installState === "cancelling";

  return (
    <div hidden={!visible}>
      <header className="topbar">
        <div>
          <span className="eyebrow">VOICE AND TEXT COMMANDS</span>
          <h1>Voice Control</h1>
          <p className="page-message">
            Commands enter one backend-owned policy, approval, execution, and
            audit gateway.
          </p>
        </div>
      </header>

      <section className="summary-grid">
        <article className="summary-card">
          <span>PC Control</span>
          <strong>{pcAgent?.status ?? "Unavailable"}</strong>
          <small>{permissionLabel}</small>
        </article>
        <article className="summary-card">
          <span>Approval policy</span>
          <strong>{pcAgent?.approvals.system ?? "deny"}</strong>
          <small>Evaluated by the backend for every action</small>
        </article>
        <article className="summary-card">
          <span>Lucy</span>
          <strong>{codingAgent ? "Ready" : "Unavailable"}</strong>
          <small>Backend resolves the active coding template</small>
        </article>
        <article className="summary-card">
          <span>Voice state</span>
          <strong>{voiceState.replace("VOICE_", "")}</strong>
          <small>
            {voiceState === "VOICE_OFF"
              ? "Microphone disabled"
              : voiceState === "VOICE_PASSIVE"
                ? `Waiting for ${preferences.voiceWakePhrase}`
                : "Accepting commands"}
          </small>
        </article>
        <article className="summary-card">
          <span>Command status</span>
          <strong>{voiceUiState}</strong>
          <small>Authoritative gateway lifecycle</small>
        </article>
      </section>

      {isDesktopRuntime() && (
        <section className="settings-note">
          Closing the window keeps the control center running in the system
          tray. Offline voice uses PipeWire and processes speech locally.
          Transcripts are interpreted in the renderer but are not written to
          the action audit.
        </section>
      )}

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">COMMAND CENTER</span>
            <h2>Speak or type a request</h2>
          </div>
          <div className="button-row">
            {isDesktopRuntime() && !voiceRuntime?.installed && (
              <button
                className="secondary-button"
                onClick={() => void installOfflineVoice()}
                disabled={voiceInstallActive}
              >
                {voiceInstallActive
                  ? "Voice installation active"
                  : "Install offline voice engine"}
              </button>
            )}
            {isDesktopRuntime() &&
              voiceRuntime?.installed &&
              !voiceRuntime.highAccuracyAvailable && (
                <button
                  className="secondary-button"
                  onClick={() => void installHighAccuracyVoice()}
                  disabled={voiceInstallActive}
                >
                  Install high-accuracy voice
                </button>
              )}
            {isDesktopRuntime() && voiceRuntime?.canCancel && (
              <button
                className="secondary-button"
                onClick={() => void cancelVoiceInstall()}
                disabled={!voiceRuntime.operationId}
              >
                Cancel voice installation
              </button>
            )}
            {isDesktopRuntime() && (
              <button
                className="secondary-button"
                onClick={toggleBackgroundVoice}
              >
                {preferences.backgroundVoiceEnabled
                  ? "Disable wake listener"
                  : "Enable wake listener"}
              </button>
            )}
            {isDesktopRuntime() && (
              <button
                className="secondary-button"
                onClick={toggleLucyMaster}
              >
                {preferences.voiceControlMasterEnabled
                  ? "Disable Lucy completely"
                  : "Enable Lucy"}
              </button>
            )}
            <button
              className="primary-button microphone-button"
              onClick={isListening ? stopListening : startListening}
              disabled={
                !preferences.voiceControlMasterEnabled ||
                (isDesktopRuntime() &&
                  (!voiceRuntime?.installed || voiceInstallActive))
              }
            >
              <span
                className={`microphone-indicator ${
                  voiceState === "VOICE_ACTIVE"
                    ? "is-active"
                    : isListening
                      ? "is-passive"
                      : "is-off"
                }`}
                aria-hidden="true"
              />
              {isListening ? "Stop listening" : "Listen"}
            </button>
          </div>
        </div>
        <div className="model-composer">
          <label className="form-field">
            <span>Command</span>
            <input
              value={command}
              onChange={(event) => setCommand(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !gatewayBusy) submitCommand();
              }}
              placeholder="Firefox, close active window, or Lucy fix the build"
            />
          </label>
          <button
            className="primary-button"
            onClick={() => submitCommand()}
            disabled={gatewayBusy}
          >
            {gatewayBusy ? "Submitting..." : "Run command"}
          </button>
        </div>
        {pendingSubmission && (
          <div className="button-row">
            <button
              className="secondary-button"
              onClick={() => void submitGatewayRequest(pendingSubmission)}
              disabled={gatewayBusy}
            >
              Retry approved action
            </button>
            <span className="form-hint">
              This retry uses the same request ID and cannot silently target a
              different action.
            </span>
          </div>
        )}
        {message && (
          <div className="runtime-message" aria-live="polite">
            {message}
          </div>
        )}
        {isDesktopRuntime() && desktopControl && (
          <p className="form-hint">{desktopControl.message}</p>
        )}
        {isDesktopRuntime() && !desktopControl?.enabled && (
          <button
            className="secondary-button"
            onClick={() => void enableDesktopControl()}
            disabled={pcAgent?.capabilities.system !== "full"}
          >
            Enable KDE desktop input
          </button>
        )}
        {isDesktopRuntime() && desktopControl?.enabled && (
          <button
            className="secondary-button"
            onClick={() => void disableDesktopControl()}
          >
            Disable KDE desktop input
          </button>
        )}
        {isDesktopRuntime() && voiceRuntime && (
          <p className="form-hint">
            {voiceRuntime.message}
            {voiceRuntime.highAccuracyAvailable
              ? " Whisper base.en transcribes commands after Lucy wakes."
              : " The Vosk base engine remains available; high accuracy is optional."}
            {preferences.backgroundVoiceEnabled &&
            preferences.voiceControlMasterEnabled
              ? " Lucy waits for its wake phrase while this app is in the tray."
              : ""}
          </p>
        )}
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">ACTION AUDIT</span>
            <h2>Recent authoritative outcomes</h2>
          </div>
          {isDesktopRuntime() && (
            <button
              className="secondary-button"
              onClick={() => void refreshAudits(true)}
            >
              Refresh audit
            </button>
          )}
        </div>
        <div className="agent-list">
          {audits.length === 0 && (
            <p className="empty-state">No system-action audit records yet.</p>
          )}
          {audits.map((audit) => (
            <article className="agent-card" key={audit.id}>
              <div>
                <h3>
                  {audit.intentKind} · {audit.status}
                </h3>
                <p>
                  {audit.detailMessage ??
                    "The backend recorded this gateway transition."}
                </p>
                <small>
                  {audit.targetKind}: {audit.targetId} · {audit.riskClass} ·{" "}
                  {new Date(audit.updatedAtUnixMs).toLocaleString()}
                </small>
                {audit.contentSha256 && (
                  <small>
                    Redacted content: {audit.contentLength ?? 0} bytes · SHA-256{" "}
                    {audit.contentSha256.slice(0, 12)}…
                  </small>
                )}
              </div>
            </article>
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">LUCY PHRASES</span>
            <h2>Wake and command vocabulary</h2>
          </div>
        </div>
        <div className="form-grid">
          <label className="form-field">
            <span>Wake phrase</span>
            <input
              value={preferences.voiceWakePhrase}
              onChange={(event) =>
                updateVoicePreference("voiceWakePhrase", event.target.value)
              }
              placeholder="lucy activate"
            />
          </label>
          <label className="form-field">
            <span>Deactivate phrase</span>
            <input
              value={preferences.voiceDeactivatePhrase}
              onChange={(event) =>
                updateVoicePreference(
                  "voiceDeactivatePhrase",
                  event.target.value,
                )
              }
              placeholder="lucy deactivate"
            />
          </label>
          <label className="form-field">
            <span>Open phrases</span>
            <input
              value={preferences.voiceOpenPhrases}
              onChange={(event) =>
                updateVoicePreference("voiceOpenPhrases", event.target.value)
              }
              placeholder="open, launch, start"
            />
          </label>
          <label className="form-field">
            <span>Close phrases</span>
            <input
              value={preferences.voiceClosePhrases}
              onChange={(event) =>
                updateVoicePreference("voiceClosePhrases", event.target.value)
              }
              placeholder="close, quit, exit"
            />
          </label>
          <label className="form-field">
            <span>Recognition replacements</span>
            <textarea
              value={preferences.voiceCommandReplacements}
              onChange={(event) =>
                updateVoicePreference(
                  "voiceCommandReplacements",
                  event.target.value,
                )
              }
              placeholder="fire fox = firefox"
            />
          </label>
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">PERMISSION LADDER</span>
            <h2>Backend-owned system-control boundaries</h2>
          </div>
        </div>
        <div className="agent-list">
          <article className="agent-card">
            <div>
              <h3>None</h3>
              <p>The backend rejects desktop actions.</p>
            </div>
          </article>
          <article className="agent-card">
            <div>
              <h3>Minor</h3>
              <p>
                Exact XDG desktop entries and standard folders remain subject
                to backend policy.
              </p>
            </div>
          </article>
          <article className="agent-card">
            <div>
              <h3>Elevated</h3>
              <p>
                Does not bypass exact-target resolution or one-use approvals.
              </p>
            </div>
          </article>
          <article className="agent-card">
            <div>
              <h3>
                Full {pcAgent?.capabilities.system === "full" ? "- Active" : ""}
              </h3>
              <p>
                Allows explicitly authorized KDE input actions. Close, Cut, and
                Delete still require one-use approval.
              </p>
              <button
                className="secondary-button"
                onClick={() => void enableDesktopControl()}
                disabled={
                  !pcAgent ||
                  pcAgent.capabilities.system !== "full" ||
                  desktopControl?.enabled
                }
              >
                {desktopControl?.enabled
                  ? "KDE desktop input active"
                  : pcAgent?.capabilities.system === "full"
                    ? "Request KDE desktop input"
                    : "Configure Full permission in Agents"}
              </button>
            </div>
          </article>
        </div>
      </section>
    </div>
  );
}
