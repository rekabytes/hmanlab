// Package ui is the Bubble Tea program for the hmanlab TUI. v0 is a
// walking skeleton per tui/docs/development/PRD.md: single chat pane,
// streaming chat with Ollama Cloud, basic markdown rendering, slash
// commands, first-run connect modal.
//
// Layout mirrors the cli's `chat` column (cli/src/ui/chat/messages.rs)
// as closely as Bubble Tea's Elm-style View model allows. The biggest
// divergence: Bubble Tea has no buffer-mutation post-render (no
// equivalent of ratatui's `f.buffer_mut()`), so hover overlays,
// selection rectangles, and per-cell animated tile backgrounds from
// the cli are deferred to v0.2+.
package ui

import (
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/charmbracelet/bubbles/key"
	"github.com/charmbracelet/bubbles/textarea"
	"github.com/charmbracelet/bubbles/textinput"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/charmbracelet/x/ansi"

	"github.com/hmanlab/hmanlab/tui/internal/config"
	"github.com/hmanlab/hmanlab/tui/internal/llm"
	"github.com/hmanlab/hmanlab/tui/internal/session"
	"github.com/hmanlab/hmanlab/tui/internal/ui/theme"
)

// Mode tracks which top-level view is on screen. Mirrors cli's
// Mode enum (cli/src/app/state.rs:196) but only the v0 subset.
type Mode int

const (
	ModeLoading Mode = iota
	ModeConnect
	ModeChat
)

// sidebarWidth is the fixed width of the right sessions panel. The
// sidebar auto-hides on terminals narrower than minChatWidth + sidebarWidth.
const sidebarWidth = 30

// sidebarVisible returns true when the terminal is wide enough to show
// the sidebar alongside a usable chat column.
func sidebarVisible(totalW int) bool {
	return totalW >= sidebarWidth+40
}

// chatWidth returns the width available to the chat column (excluding
// the sidebar + 1-col gap when visible).
func (m Model) chatWidth() int {
	if sidebarVisible(m.width) {
		return m.width - sidebarWidth - 1 // -1 gap between chat and sidebar
	}
	return m.width
}

// Model is the top-level Bubble Tea state. Everything the TUI knows
// lives on this struct — Update is pure dispatch, View is pure render.
type Model struct {
	mode Mode

	cfg      *config.Config
	provider llm.Provider
	model    string

	// cwd is the working directory the TUI was launched from. Used to
	// group sessions per-project on disk.
	cwd string

	// activeSession tracks the JSONL file we're appending to. nil until
	// the first user message creates one (lazy init).
	activeSession *session.Active

	// Session picker overlay state (toggled by /sessions, /load).
	sessionList    []session.SessionSummary
	sessionCursor  int
	sessionOverlay bool

	// Model picker overlay state (toggled by /model with no arg).
	modelEntries []modelEntry
	modelCursor  int
	modelOverlay bool

	// API key input modal — opened when selecting an unconfigured
	// provider from the model picker.
	keyInputProvider string
	keyInputActive   bool
	keyInput         textinput.Model

	// Slash command autocomplete dropdown state. Shows when the input
	// starts with "/" and no space has been typed yet.
	cmdDropdown bool
	cmdMatches  []commandInfo
	cmdCursor   int

	// lastKeyWasFiltered is set when a terminal OSC response fragment
	// is swallowed, so subsequent fragments (], \, ;, digits, etc.)
	// can be caught too. OSC responses arrive as a rapid chain of
	// short key events that Bubble Tea couldn't parse as one sequence.
	lastKeyWasFiltered  bool
	filterChainCount    int

	// loadingTicks counts ticks during the startup loading screen.
	// After loadingMaxTicks, we transition to chat/connect.
	loadingTicks int

	// cleanupTicks counts down after loading finishes. While > 0, we
	// check the textarea on each tick and clear it if it contains
	// only OSC garbage (non-letter characters). This catches fragments
	// that slip through the event-level filter.
	cleanupTicks int

	// Right sidebar — persistent session list + token stats.
	sidebarSessions   []session.SessionSummary
	sidebarCursor     int
	sidebarFocus      bool
	totalPromptTokens int
	totalCompletionTokens int

	history []chatMessage
	input   textarea.Model
	viewport viewport.Model

	// connect holds the first-run modal state when mode == ModeConnect.
	connect connectModal

	// stream management for the in-flight assistant response.
	streamCtx     context.Context
	streamCancel  context.CancelFunc
	streamCh      <-chan llm.StreamEvent
	streaming     bool   // true iff an assistant response is being streamed
	streamBuffer  string // pending text not yet flushed to content (word-by-word pacing)
	streamDone    bool   // channel closed but buffer may still have text

	// animTick drives the blinking caret on the streaming message.
	// Incremented every ~120ms via tickMsg.
	animTick uint64

	// width/height are the latest known terminal dimensions, used for
	// markdown re-wrap + viewport sizing.
	width  int
	height int

	// followTail pins the viewport to the bottom while streaming and
	// flips off on manual scroll-up.
	followTail bool

	// statusLine is the bottom-of-screen one-liner (model, tokens, etc.).
	statusLine string

	// lastPromptTokens + lastCompletionTokens drive the status bar's
	// per-turn token counts. Updated when a stream finishes (Done event).
	lastPromptTokens     int
	lastCompletionTokens int

	// quitErr carries a fatal error that should be displayed on exit
	// (rare; most errors surface inline in chat instead).
	quitErr error
}

// New constructs the initial Model. The caller (cmd/hmanlab-tui/main.go)
// passes in a loaded config + the current working directory (used for
// session grouping). If the config has no Ollama Cloud key, we boot
// straight into ModeConnect for the first-run flow.
func New(cfg *config.Config, cwd string) Model {
	ta := textarea.New()
	ta.Placeholder = "Send a message…"
	ta.Prompt = ""
	ta.ShowLineNumbers = false
	ta.CharLimit = 0
	ta.SetHeight(3)
	ta.KeyMap.InsertNewline = key.NewBinding(key.WithKeys("enter", "alt+enter", "ctrl+m"))
	// Match the textarea's own backgrounds to the input-area wrapper
	// (BG_CHAT) so the beam + textarea read as one continuous surface
	// instead of two stacked boxes. The cursor line stays unstyled so
	// there's no jarring highlight on the active row.
	ta.FocusedStyle.Base = lipgloss.NewStyle().Background(theme.BGChat)
	ta.FocusedStyle.CursorLine = lipgloss.NewStyle().Background(theme.BGChat)
	ta.FocusedStyle.Placeholder = lipgloss.NewStyle().Background(theme.BGChat).Foreground(theme.FGDim)
	ta.FocusedStyle.Text = lipgloss.NewStyle().Background(theme.BGChat).Foreground(theme.FG)
	ta.FocusedStyle.EndOfBuffer = lipgloss.NewStyle().Background(theme.BGChat)
	ta.FocusedStyle.Prompt = lipgloss.NewStyle().Background(theme.BGChat)
	ta.BlurredStyle.Base = lipgloss.NewStyle().Background(theme.BGChat)
	ta.BlurredStyle.CursorLine = lipgloss.NewStyle().Background(theme.BGChat)
	ta.BlurredStyle.Placeholder = lipgloss.NewStyle().Background(theme.BGChat).Foreground(theme.FGDim)
	ta.BlurredStyle.Text = lipgloss.NewStyle().Background(theme.BGChat).Foreground(theme.FG)
	ta.BlurredStyle.EndOfBuffer = lipgloss.NewStyle().Background(theme.BGChat)
	ta.BlurredStyle.Prompt = lipgloss.NewStyle().Background(theme.BGChat)
	ta.Focus()

	vp := viewport.New(80, 20)
	vp.SetContent("")

	ki := textinput.New()
	ki.Prompt = ""
	ki.Placeholder = "Paste your API key"
	ki.CharLimit = 256
	ki.EchoMode = textinput.EchoPassword

	m := Model{
		cfg:          cfg,
		cwd:          cwd,
		input:        ta,
		viewport:     vp,
		followTail:   true,
		mode:         ModeLoading,
		keyInput:     ki,
	}

	return m
}

// finishLoading transitions from the loading screen to either chat
// (if a key is configured) or the connect modal. Clears any OSC
// garbage that may have leaked into the textarea during startup.
func (m *Model) finishLoading() {
	m.input.Reset()
	m.lastKeyWasFiltered = false
	m.filterChainCount = 0
	m.cleanupTicks = 5 // ~600ms post-loading garbage cleanup
	if m.cfg.HasOllamaCloudKey() {
		m.bootIntoChat()
		m.relayout()
		m.refreshViewportContent()
	} else {
		m.mode = ModeConnect
		m.connect = newConnectModal()
	}
}

// bootIntoChat wires up the provider + model from config and switches
// to ModeChat. Used both at startup (when a key is already configured)
// and after the connect modal successfully validates a freshly entered key.
func (m *Model) bootIntoChat() {
	m.model = m.cfg.EffectiveModel()
	if m.cfg.LastProvider == "" {
		m.cfg.LastProvider = config.OllamaCloudProvider
	}
	m.provider = llm.BackendFor(m.cfg, m.cfg.LastProvider)
	if m.provider == nil {
		// Fallback: try Ollama Cloud.
		m.provider = llm.NewCloudOllama(m.cfg.OllamaCloudAPIKey)
		m.cfg.LastProvider = config.OllamaCloudProvider
	}
	m.mode = ModeChat
	m.statusLine = "ready"
	m.refreshSidebar()
	m.history = []chatMessage{
		infoLine(fmt.Sprintf("Connected to **%s** (%s). Try /help for commands.", config.ProviderLabel(m.cfg.LastProvider), m.model)),
	}
}

// Init implements tea.Model. Returns the commands we want running on
// startup: a ticker for the streaming-caret animation.
func (m Model) Init() tea.Cmd {
	return tea.Batch(
		tickCmd(),
		textarea.Blink, // start the input cursor blinking
	)
}

// tickCmd is the per-frame animation driver. Returns a tea.Cmd that
// sleeps ~120ms (matches the cli's anim_tick cadence) and emits a
// tickMsg. Each tick bumps m.animTick by 1, which the View uses to
// blink the caret on streaming messages.
type tickMsg struct{}

func tickCmd() tea.Cmd {
	return tea.Tick(120*time.Millisecond, func(time.Time) tea.Msg {
		return tickMsg{}
	})
}

// Update implements tea.Model.
func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	// Guard: filter terminal OSC responses before any mode-specific
	// handling. This runs during ALL modes (including loading) so the
	// filter flag is set properly even when the response straddles the
	// loading→chat transition.
	if k, ok := msg.(tea.KeyMsg); ok {
		s := k.String()
		prevFiltered := m.lastKeyWasFiltered
		m.lastKeyWasFiltered = false
		m.filterChainCount = 0
		if isTerminalResponse(s) {
			m.lastKeyWasFiltered = true
			return m, nil
		}
		if prevFiltered && m.filterChainCount < 15 && isOSCFragment(s) {
			m.lastKeyWasFiltered = true
			m.filterChainCount++
			return m, nil
		}
	}

	switch v := msg.(type) {
	case tea.WindowSizeMsg:
		m.width, m.height = int(v.Width), int(v.Height)
		m.relayout()
		return m, nil

	case tickMsg:
		m.animTick++
		// Post-loading garbage cleanup: if the textarea picked up OSC
		// fragments that slipped through the event filter, clear it.
		if m.cleanupTicks > 0 {
			m.cleanupTicks--
			if val := m.input.Value(); val != "" && !m.streaming && looksLikeGarbage(val) {
				m.input.Reset()
			}
		}
		// Loading screen: count ticks, then transition to chat/connect.
		// This grace period lets terminal OSC responses arrive and be
		// swallowed before the user can type.
		if m.mode == ModeLoading {
			m.loadingTicks++
			if m.loadingTicks >= loadingMaxTicks {
				m.finishLoading()
			}
			return m, tickCmd()
		}
		// Drain the stream channel alongside the tick — gives us
		// ~120ms-granularity polling on the in-flight response. Cheaper
		// than spawning a tea.Cmd per chunk and avoids a backlog when
		// the model emits tokens faster than we can re-render.
		if m.streaming {
			cmds := m.drainStream()
			return m, tea.Batch(append(cmds, tickCmd())...)
		}
		return m, tickCmd()

	case connectAttemptMsg:
		// User submitted the connect modal. Validate via the provider's
		// Ping (one GET /api/tags). Run as a tea.Cmd so the UI doesn't
		// block; result lands as connectResultMsg.
		key := v.Key
		return m, func() tea.Msg {
			probe := llm.NewCloudOllama(key)
			ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
			defer cancel()
			return connectResultMsg{Err: probe.Ping(ctx)}
		}

	case connectResultMsg:
		if v.Err != nil {
			// Stay in modal; modal will surface the error.
			mod, _, _ := m.connect.Update(v, m.width, m.height)
			m.connect = mod
			return m, nil
		}
		// Success — persist key + boot into chat.
		m.cfg.OllamaCloudAPIKey = strings.TrimSpace(m.connect.input.Value())
		m.cfg.LastProvider = config.OllamaCloudProvider
		if err := m.cfg.Save(); err != nil {
			// Non-fatal — key still works for this session, just won't
			// be remembered. Surface as a one-off system message in
			// the chat history once we switch modes.
			m.bootIntoChat()
			m.history = append(m.history, infoLine(fmt.Sprintf(
				"⚠ connected, but couldn't save config: %v", err)))
			return m, nil
		}
		m.bootIntoChat()
		return m, nil

	case streamDoneMsg:
		// Stream finished (Done event from drainStream). Reset state.
		m.streaming = false
		m.streamBuffer = ""
		m.streamDone = false
		if m.streamCancel != nil {
			m.streamCancel()
			m.streamCancel = nil
		}
		m.streamCh = nil
		// Mark the trailing message as no longer streaming + persist it.
		for i := len(m.history) - 1; i >= 0; i-- {
			if m.history[i].streaming {
				m.history[i].streaming = false
				if v.cancelled {
					m.history[i].cancelled = true
				}
				// Save the completed (or cancelled) reply to the session.
				content := m.history[i].content
				if v.cancelled && !strings.HasSuffix(content, "[...]") {
					content += " [...]"
				}
				if content != "" {
					if err := m.writeAssistantToSession(content); err != nil {
						m.history = append(m.history, infoLine(
							fmt.Sprintf("⚠ couldn't save reply: %v", err)))
					}
				}
				break
			}
		}
		m.lastPromptTokens = v.promptTokens
		m.lastCompletionTokens = v.completionTokens
		m.totalPromptTokens += v.promptTokens
		m.totalCompletionTokens += v.completionTokens
		m.refreshSidebar()
		m.statusLine = ""
		m.refreshViewportContent()
		return m, nil
	}

	// Mode-specific routing.
	switch m.mode {
	case ModeLoading:
		return m, nil // swallow all input during loading
	case ModeConnect:
		return m.updateConnect(msg)
	case ModeChat:
		return m.updateChat(msg)
	}
	return m, nil
}

// updateConnect forwards every msg to the modal.
func (m Model) updateConnect(msg tea.Msg) (tea.Model, tea.Cmd) {
	mod, cmd, done := m.connect.Update(msg, m.width, m.height)
	m.connect = mod
	if done {
		// Modal closed. If we got here via tea.Quit (Esc / Ctrl+C),
		// bubbletea will quit on its own; nothing to do.
		return m, cmd
	}
	return m, cmd
}

// updateChat handles the chat-view key tree.
func (m Model) updateChat(msg tea.Msg) (tea.Model, tea.Cmd) {
	// Session picker overlay intercepts all keys while open.
	if m.sessionOverlay {
		return m.updateSessionOverlay(msg)
	}
	// Model picker overlay intercepts all keys while open.
	if m.modelOverlay {
		return m.updateModelOverlay(msg)
	}
	// Key input modal intercepts all keys while open.
	if m.keyInputActive {
		return m.updateKeyInput(msg)
	}
	switch k := msg.(type) {
	case tea.KeyMsg:
		// OSC filtering is handled in the common Update() guard above.
		// Slash command dropdown: intercept navigation keys while open.
		if m.cmdDropdown && len(m.cmdMatches) > 0 {
			switch k.Type {
			case tea.KeyUp:
				if m.cmdCursor > 0 {
					m.cmdCursor--
				}
				return m, nil
			case tea.KeyDown:
				if m.cmdCursor < len(m.cmdMatches)-1 {
					m.cmdCursor++
				}
				return m, nil
			case tea.KeyTab:
				// Autocomplete the command name, let user type args.
				selected := m.cmdMatches[m.cmdCursor]
				m.input.SetValue(selected.display + " ")
				m.input.CursorEnd()
				m.cmdDropdown = false
				return m, nil
			case tea.KeyEnter:
				if !m.streaming {
					selected := m.cmdMatches[m.cmdCursor]
					m.input.Reset()
					m.cmdDropdown = false
					return m.handleSlashCommand(slashCommand{kind: selected.kind})
				}
			case tea.KeyEsc:
				m.cmdDropdown = false
				return m, nil
			}
		}
		// Tab: toggle sidebar focus (only when dropdown closed, not
		// streaming, and the sidebar is visible).
		if k.Type == tea.KeyTab && !m.cmdDropdown && !m.streaming && sidebarVisible(m.width) {
			m.sidebarFocus = !m.sidebarFocus
			if m.sidebarFocus {
				m.input.Blur()
				m.refreshSidebar()
				return m, nil
			}
			m.input.Focus()
			return m, textarea.Blink
		}
		// Sidebar navigation when focused.
		if m.sidebarFocus {
			switch k.Type {
			case tea.KeyUp:
				if m.sidebarCursor > 0 {
					m.sidebarCursor--
				}
				return m, nil
			case tea.KeyDown:
				if m.sidebarCursor < len(m.sidebarSessions)-1 {
					m.sidebarCursor++
				}
				return m, nil
			case tea.KeyEnter:
				if m.sidebarCursor < len(m.sidebarSessions) {
					sum := m.sidebarSessions[m.sidebarCursor]
					if err := m.loadSession(&sum); err != nil {
						m.history = append(m.history, infoLine(fmt.Sprintf("✕ couldn't load: %v", err)))
					}
				}
				m.sidebarFocus = false
				m.input.Focus()
				return m, textarea.Blink
			case tea.KeyEsc:
				m.sidebarFocus = false
				m.input.Focus()
				return m, textarea.Blink
			}
		}
		// Ctrl+C: cancel in-flight stream, or quit if idle on empty input.
		if k.Type == tea.KeyCtrlC {
			if m.streaming {
				m.cancelStream(true)
				return m, nil
			}
			if m.input.Value() == "" {
				return m, tea.Quit
			}
			// Otherwise: clear the input rather than quit — matches the
			// cli's "Ctrl+C is interrupt, not exit" UX.
			m.input.Reset()
			m.cmdDropdown = false
			return m, nil
		}
		// Ctrl+D on empty input: quit.
		if k.Type == tea.KeyCtrlD && m.input.Value() == "" {
			return m, tea.Quit
		}
		// Page up/down scroll the history viewport.
		if k.Type == tea.KeyPgUp {
			m.viewport.LineUp(m.viewport.Height / 2)
			m.followTail = false
			return m, nil
		}
		if k.Type == tea.KeyPgDown {
			m.viewport.LineDown(m.viewport.Height / 2)
			if m.viewport.AtBottom() {
				m.followTail = true
			}
			return m, nil
		}
		// Mouse wheel → viewport scroll. bubbletea delivers mouse
		// events as tea.MouseMsg (not KeyMsg) — handled by the
		// default branch forwarding to viewport.Update.

		// Enter (without Alt): send. Alt+Enter falls through to the
		// textarea which inserts a newline.
		if k.Type == tea.KeyEnter && !k.Alt && !m.streaming {
			text := strings.TrimSpace(m.input.Value())
			if text == "" {
				return m, nil
			}
			// Slash command? Dispatch.
			if cmd, ok := parseSlashCommand(text); ok {
				return m.handleSlashCommand(cmd)
			}
			// Regular message: append user turn, open stream.
			return m.sendUserMessage(text)
		}
	}

	// Default: forward to textarea + viewport.
	var cmd1, cmd2 tea.Cmd
	m.input, cmd1 = m.input.Update(msg)
	m.viewport, cmd2 = m.viewport.Update(msg)
	m.refreshCmdDropdown() // update dropdown state FIRST
	m.relayout()           // THEN relayout with correct dropdownH
	return m, tea.Batch(cmd1, cmd2)
}

// refreshCmdDropdown checks the current input value and shows/hides the
// slash command autocomplete dropdown. The dropdown appears when the
// input starts with "/" and contains no spaces (still typing the command
// name). It disappears once a space is typed (user is entering args) or
// the input no longer starts with "/".
func (m *Model) refreshCmdDropdown() {
	val := m.input.Value()
	if !strings.HasPrefix(val, "/") {
		m.cmdDropdown = false
		return
	}
	// Only autocomplete while typing the head word (no space yet).
	if strings.ContainsAny(val, " \t") {
		m.cmdDropdown = false
		return
	}
	query := strings.ToLower(val)
	var matches []commandInfo
	for _, c := range allCommands() {
		if strings.HasPrefix(strings.ToLower(c.display), query) {
			matches = append(matches, c)
		}
	}
	if len(matches) == 0 || (len(matches) == 1 && strings.EqualFold(matches[0].display, val)) {
		// Exact match already typed — no need to show the dropdown.
		m.cmdDropdown = false
		return
	}
	m.cmdDropdown = true
	m.cmdMatches = matches
	if m.cmdCursor >= len(m.cmdMatches) {
		m.cmdCursor = len(m.cmdMatches) - 1
	}
	if m.cmdCursor < 0 {
		m.cmdCursor = 0
	}
}

// sendUserMessage appends a user turn, opens a streaming request, and
// returns the cmds needed to start polling the stream.
func (m Model) sendUserMessage(text string) (tea.Model, tea.Cmd) {
	m.history = append(m.history, chatMessage{
		role:    llm.RoleUser,
		content: text,
	})
	m.input.Reset()

	// Persist to the session JSONL. Create the session lazily on the
	// first user message (meta record + title), then append the user
	// record.
	if err := m.ensureSessionAndWriteUser(text); err != nil {
		m.history = append(m.history, infoLine(fmt.Sprintf("⚠ couldn't save session: %v", err)))
	}

	// Build the message slice we send to the provider — full history,
	// excluding the leading info line and any cancelled/empty messages.
	apiMsgs := []llm.Message{
		{Role: string(llm.RoleSystem), Content: workspaceSystemPrompt()},
	}
	for _, h := range m.history {
		if h.role == llm.RoleSystem {
			continue
		}
		if h.content == "" {
			continue
		}
		apiMsgs = append(apiMsgs, llm.Message{Role: string(h.role), Content: h.content})
	}

	// Open the stream with a cancellable context.
	ctx, cancel := context.WithCancel(context.Background())
	m.streamCtx, m.streamCancel = ctx, cancel
	m.streamCh = m.provider.StreamChat(ctx, m.model, apiMsgs, nil)
	m.streaming = true
	m.streamBuffer = ""
	m.streamDone = false

	// Push a placeholder assistant message; tokens will accumulate into it.
	m.history = append(m.history, chatMessage{
		role:      llm.RoleAssistant,
		streaming: true,
	})

	m.followTail = true
	m.statusLine = "" // streaming indicator is rendered separately
	m.refreshViewportContent()
	// Kick the polling loop. tickCmd will see m.streaming=true and
	// drain the channel on the next tick.
	return m, tickCmd()
}

// handleSlashCommand dispatches a parsed slash command. Returns the
// updated model + any cmd (e.g. tea.Quit for /quit).
func (m Model) handleSlashCommand(cmd slashCommand) (tea.Model, tea.Cmd) {
	m.cmdDropdown = false
	switch cmd.kind {
	case "help":
		m.history = append(m.history, chatMessage{
			role:    llm.RoleAssistant,
			content: helpText(m.model),
		})
		m.input.Reset()
		m.followTail = true
		m.refreshViewportContent()
		return m, nil
	case "clear":
		m.activeSession = nil // next message starts a fresh session
		m.totalPromptTokens = 0
		m.totalCompletionTokens = 0
		m.refreshSidebar()
		m.history = []chatMessage{
			infoLine("History cleared."),
		}
		m.input.Reset()
		m.refreshViewportContent()
		return m, nil
	case "quit":
		return m, tea.Quit
	case "model":
		arg := strings.TrimSpace(cmd.arg)
		if arg == "" {
			// Open the model picker overlay.
			m.modelEntries = m.buildModelList()
			m.modelCursor = 0
			m.modelOverlay = true
			m.input.Reset()
			return m, nil
		}
		// Validate against all configured providers' catalogs.
		var foundProvider string
		for _, p := range m.cfg.ConfiguredProviders() {
			for _, name := range config.ProviderModels(p) {
				if name == arg {
					foundProvider = p
					break
				}
			}
			if foundProvider != "" {
				break
			}
		}
		if foundProvider == "" {
			m.history = append(m.history, infoLine(fmt.Sprintf("✕ unknown model %q. Use /model to see available models.", arg)))
		} else {
			m.model = arg
			m.cfg.LastModel = arg
			m.cfg.LastProvider = foundProvider
			_ = m.cfg.Save()
			if backend := llm.BackendFor(m.cfg, foundProvider); backend != nil {
				m.provider = backend
			}
			m.history = append(m.history, infoLine(fmt.Sprintf("✓ switched to %s (%s)", arg, config.ProviderLabel(foundProvider))))
		}
		m.input.Reset()
		m.followTail = true
		m.refreshViewportContent()
		return m, nil
	case "sessions":
		return m.openSessionPicker()
	case "load":
		prefix := strings.TrimSpace(cmd.arg)
		if prefix != "" {
			return m.loadSessionByPrefix(prefix)
		}
		return m.openSessionPicker()
	case "unknown":
		m.history = append(m.history, infoLine(fmt.Sprintf("✕ unknown command: /%s (try /help)", cmd.arg)))
		m.input.Reset()
		m.refreshViewportContent()
		return m, nil
	}
	return m, nil
}

// streamDoneMsg is the internal "stream finished" signal. Emitted by
// drainStream when the channel closes or the Done event arrives.
type streamDoneMsg struct {
	promptTokens     int
	completionTokens int
	cancelled        bool
}

// drainStream pulls all currently-buffered events off the stream
// channel without blocking. Returns a slice of cmds: typically empty
// if the channel has nothing ready, or a single streamDoneMsg if the
// stream just closed.
func (m *Model) drainStream() []tea.Cmd {
	if m.streamCh == nil {
		return nil
	}
	var cmds []tea.Cmd

	// Drain all available events into the buffer.
drainLoop:
	for {
		select {
		case ev, ok := <-m.streamCh:
			if !ok {
				m.streamDone = true
				break drainLoop
			}
			if ev.Err != nil {
				m.history = append(m.history, infoLine(fmt.Sprintf("✕ error: %s", ev.Err.Error())))
				m.streamDone = true
				break drainLoop
			}
			if ev.Done {
				m.streamDone = true
				m.lastPromptTokens = ev.PromptTokens
				m.lastCompletionTokens = ev.CompletionTokens
				break drainLoop
			}
			if ev.Text != "" {
				m.streamBuffer += ev.Text
			}
		default:
			break drainLoop
		}
	}

	// Flush one word from the buffer to the streaming message content.
	// This paces the output to one word per ~120ms tick — a humanized
	// typewriter effect instead of dumping the whole response at once.
	if m.streamBuffer != "" {
		var word string
		idx := strings.IndexAny(m.streamBuffer, " \n\t")
		if idx == -1 {
			// No word boundary yet — only flush if stream is done
			// (no more text coming to complete the word).
			if m.streamDone {
				word = m.streamBuffer
				m.streamBuffer = ""
			}
		} else {
			word = m.streamBuffer[:idx+1]
			m.streamBuffer = m.streamBuffer[idx+1:]
		}
		if word != "" {
			for i := len(m.history) - 1; i >= 0; i-- {
				if m.history[i].streaming {
					m.history[i].content += word
					break
				}
			}
		}
	}

	// Only finalize when the stream is done AND the buffer is empty.
	if m.streamDone && m.streamBuffer == "" {
		cmds = append(cmds, func() tea.Msg {
			return streamDoneMsg{
				promptTokens:     m.lastPromptTokens,
				completionTokens: m.lastCompletionTokens,
			}
		})
	}

	if m.followTail {
		m.refreshViewportContent()
	}
	return cmds
}

// cancelStream aborts the in-flight request. If markCancelled is true,
// the trailing streaming message gets a trailing […] marker so the user
// sees the response was truncated rather than complete. The partial
// reply is persisted to the session so it survives a reload.
func (m *Model) cancelStream(markCancelled bool) {
	if !m.streaming {
		return
	}
	if m.streamCancel != nil {
		m.streamCancel()
	}
	m.streaming = false
	m.streamBuffer = ""
	m.streamDone = false
	for i := len(m.history) - 1; i >= 0; i-- {
		if m.history[i].streaming {
			m.history[i].streaming = false
			if markCancelled {
				m.history[i].cancelled = true
			}
			// Persist whatever we got before the cancel.
			content := m.history[i].content
			if markCancelled && !strings.HasSuffix(content, "[...]") {
				content += " [...]"
			}
			if content != "" {
				if err := m.writeAssistantToSession(content); err != nil {
					m.history = append(m.history, infoLine(
						fmt.Sprintf("⚠ couldn't save reply: %v", err)))
				}
			}
			break
		}
	}
	m.statusLine = "cancelled"
	m.refreshViewportContent()
}

// ---------------------------------------------------------------------------
// Session persistence helpers
// ---------------------------------------------------------------------------

// ensureSessionAndWriteUser creates the active session (meta record) if
// none exists yet, then appends the user message. The session title is
// derived from the first line of the message.
func (m *Model) ensureSessionAndWriteUser(text string) error {
	if m.activeSession == nil {
		id := session.NewID()
		path, err := session.PathFor(m.cwd, id)
		if err != nil {
			return err
		}
		title := session.TruncateTitle(text)
		if err := session.WriteMeta(path, id, title, m.model); err != nil {
			return err
		}
		m.activeSession = &session.Active{SessionID: id, Path: path}
		// Reset cumulative token counts for the new session.
		m.totalPromptTokens = 0
		m.totalCompletionTokens = 0
		m.refreshSidebar()
	}
	return session.WriteUser(m.activeSession.Path, m.activeSession.SessionID, text, m.model)
}

// writeAssistantToSession appends a completed (or partial) assistant reply.
func (m *Model) writeAssistantToSession(content string) error {
	if m.activeSession == nil {
		return nil // no active session — nothing to persist
	}
	return session.WriteAssistant(m.activeSession.Path, m.activeSession.SessionID, content, m.model)
}

// loadSession replaces the current chat with a saved session's history.
func (m *Model) loadSession(sum *session.SessionSummary) error {
	records, err := session.LoadRecords(sum.Path)
	if err != nil {
		return err
	}
	m.history = []chatMessage{
		infoLine(fmt.Sprintf("Loaded session: **%s** (%s)", sum.Title, sum.Model)),
	}
	for _, r := range records {
		switch r.Kind {
		case session.KindUser:
			m.history = append(m.history, chatMessage{role: llm.RoleUser, content: r.Content})
		case session.KindAssistant:
			m.history = append(m.history, chatMessage{role: llm.RoleAssistant, content: r.Content})
		}
	}
	m.activeSession = &session.Active{SessionID: sum.SessionID, Path: sum.Path}
	m.totalPromptTokens = 0
	m.totalCompletionTokens = 0
	m.sidebarCursor = 0
	m.followTail = true
	m.refreshViewportContent()
	m.refreshSidebar()
	return nil
}

// refreshSidebar reloads the session list for the current project so the
// sidebar stays in sync with disk.
func (m *Model) refreshSidebar() {
	summaries, err := session.ListSessions(m.cwd)
	if err != nil || summaries == nil {
		m.sidebarSessions = nil
		return
	}
	m.sidebarSessions = summaries
	if m.sidebarCursor >= len(m.sidebarSessions) {
		m.sidebarCursor = max(0, len(m.sidebarSessions)-1)
	}
}

// openSessionPicker loads the session list and shows the overlay.
func (m Model) openSessionPicker() (tea.Model, tea.Cmd) {
	summaries, err := session.ListSessions(m.cwd)
	if err != nil {
		m.history = append(m.history, infoLine(fmt.Sprintf("✕ couldn't list sessions: %v", err)))
		m.input.Reset()
		m.refreshViewportContent()
		return m, nil
	}
	if len(summaries) == 0 {
		m.history = append(m.history, infoLine("No saved sessions for this project yet."))
		m.input.Reset()
		m.refreshViewportContent()
		return m, nil
	}
	m.sessionList = summaries
	m.sessionCursor = 0
	m.sessionOverlay = true
	m.input.Reset()
	return m, nil
}

// loadSessionByPrefix resolves a prefix arg and loads the session inline.
func (m Model) loadSessionByPrefix(prefix string) (tea.Model, tea.Cmd) {
	summaries, err := session.ListSessions(m.cwd)
	if err != nil {
		m.history = append(m.history, infoLine(fmt.Sprintf("✕ couldn't list sessions: %v", err)))
		m.input.Reset()
		m.refreshViewportContent()
		return m, nil
	}
	sum, err := session.FindByPrefix(summaries, prefix)
	if err != nil {
		m.history = append(m.history, infoLine(fmt.Sprintf("✕ %v", err)))
		m.input.Reset()
		m.refreshViewportContent()
		return m, nil
	}
	if err := m.loadSession(sum); err != nil {
		m.history = append(m.history, infoLine(fmt.Sprintf("✕ couldn't load: %v", err)))
	}
	m.input.Reset()
	return m, nil
}

// updateSessionOverlay handles keys while the session picker is open.
func (m Model) updateSessionOverlay(msg tea.Msg) (tea.Model, tea.Cmd) {
	k, ok := msg.(tea.KeyMsg)
	if !ok {
		return m, nil
	}
	// OSC filtering is handled in the common Update() guard above.
	switch k.Type {
	case tea.KeyEsc, tea.KeyCtrlC:
		m.sessionOverlay = false
		return m, nil
	case tea.KeyUp:
		if m.sessionCursor > 0 {
			m.sessionCursor--
		}
		return m, nil
	case tea.KeyDown:
		if m.sessionCursor < len(m.sessionList)-1 {
			m.sessionCursor++
		}
		return m, nil
	case tea.KeyEnter:
		if m.sessionCursor < len(m.sessionList) {
			sum := m.sessionList[m.sessionCursor]
			if err := m.loadSession(&sum); err != nil {
				m.history = append(m.history, infoLine(fmt.Sprintf("✕ couldn't load: %v", err)))
			}
		}
		m.sessionOverlay = false
		return m, nil
	}
	return m, nil
}

// updateModelOverlay handles keys while the /model picker is open.
func (m Model) updateModelOverlay(msg tea.Msg) (tea.Model, tea.Cmd) {
	k, ok := msg.(tea.KeyMsg)
	if !ok {
		return m, nil
	}
	switch k.Type {
	case tea.KeyEsc, tea.KeyCtrlC:
		m.modelOverlay = false
		return m, nil
	case tea.KeyUp:
		if m.modelCursor > 0 {
			m.modelCursor--
		}
		return m, nil
	case tea.KeyDown:
		if m.modelCursor < len(m.modelEntries)-1 {
			m.modelCursor++
		}
		return m, nil
	case tea.KeyEnter:
		if m.modelCursor < len(m.modelEntries) {
			e := m.modelEntries[m.modelCursor]
			if !e.configured {
				// Open key input modal for this provider.
				m.keyInputProvider = e.provider
				m.keyInputActive = true
				m.modelOverlay = false
				m.input.Blur()
				m.keyInput.Placeholder = "Paste your API key for " + config.ProviderLabel(e.provider)
				m.keyInput.SetValue("")
				m.keyInput.Focus()
				return m, textinput.Blink
			}
			m.model = e.model
			m.cfg.LastModel = e.model
			m.cfg.LastProvider = e.provider
			_ = m.cfg.Save()
			// Switch the backend to the selected provider.
			if backend := llm.BackendFor(m.cfg, e.provider); backend != nil {
				m.provider = backend
			}
			m.history = append(m.history, infoLine(fmt.Sprintf(
				"✓ switched to %s (%s)", e.model, config.ProviderLabel(e.provider))))
		}
		m.modelOverlay = false
		m.followTail = true
		m.refreshViewportContent()
		return m, nil
	}
	return m, nil
}

// relayout recomputes viewport + input dimensions after a window
// resize or input change. The textarea grows with content up to 15
// lines, then scrolls internally. Height split (top → bottom):
//
//	header (1) + viewport (rest) + input (1 padding + N textarea) +
//	status (1) + bottom (1).
func (m *Model) relayout() {
	if m.width < 10 || m.height < 10 {
		return
	}
	cw := m.chatWidth()
	const (
		headerH    = 1
		statusH    = 1
		bottomH    = 1 // breathing room under status bar
		gapH       = 1 // breathing room between input and status bar
		maxTaLines = 15
		minTaLines = 3
	)
	taLines := strings.Count(m.input.Value(), "\n") + 1
	if taLines > maxTaLines {
		taLines = maxTaLines
	}
	if taLines < minTaLines {
		taLines = minTaLines
	}
	inputH := taLines + 1 // +1 top padding
	// Dropdown takes space above the input when visible.
	dropdownH := 0
	if m.cmdDropdown && len(m.cmdMatches) > 0 {
		dropdownH = len(m.cmdMatches)
	}
	// -1 safety margin so the terminal doesn't scroll and push the
	// header off the top.
	vpH := m.height - headerH - inputH - statusH - bottomH - gapH - dropdownH - 1
	if vpH < 3 {
		vpH = 3
	}
	if m.mode == ModeChat {
		m.viewport.Width = cw
		m.viewport.Height = vpH
		m.input.SetHeight(taLines)
		// Textarea width = chat col - 1 (slim beam) - 1 (gap after beam).
		inputW := cw - 1 - 1
		if inputW < 10 {
			inputW = 10
		}
		m.input.SetWidth(inputW)
	}
}

// helpHint is the right-aligned affordance in the status bar (same
// row as the model name). Rendered dim + italic so it reads as a hint,
// not a primary label.
const helpHint = "/help for commands"

// refreshViewportContent re-renders the chat history and pushes it
// into the viewport. Called on every state change that affects the
// visible rows: new user message, streaming token, /clear, etc.
func (m *Model) refreshViewportContent() {
	cw := m.chatWidth()
	bodyW := cw - 4 // gutter + breathing room
	if bodyW < 10 {
		bodyW = 10
	}
	var blocks []string
	for _, h := range m.history {
		blocks = append(blocks, h.render(bodyW, cw, m.animTick))
	}
	content := strings.Join(blocks, "\n\n")
	m.viewport.SetContent(content)
	if m.followTail {
		m.viewport.GotoBottom()
	}
}

// View implements tea.Model.
func (m Model) View() string {
	if m.width == 0 || m.height == 0 {
		return "loading…"
	}
	switch m.mode {
	case ModeLoading:
		return m.viewLoading()
	case ModeConnect:
		return m.connect.View(m.width, m.height)
	case ModeChat:
		view := m.viewChat()
		if m.sessionOverlay {
			view = renderSessionOverlay(m.width, m.height, m.sessionList, m.sessionCursor)
		} else if m.modelOverlay {
			view = renderModelOverlay(m.width, m.height, m.modelEntries, m.modelCursor, m.model)
		} else if m.keyInputActive {
			view = renderKeyInputModal(m.width, m.height, m.keyInputProvider, m.keyInput)
		}
		return view
	}
	return ""
}

// viewChat renders the chat layout: left chat column + optional right
// sessions sidebar. Vertical stack per column (top → bottom): header ·
// history viewport · dropdown · input area · status bar · bottom pad.
func (m Model) viewChat() string {
	cw := m.chatWidth()

	// ── Header strip ──────────────────────────────────────────────
	mark := lipgloss.NewStyle().Foreground(theme.HibiscusGlow).Render("✿")
	wordmark := lipgloss.NewStyle().
		Foreground(theme.Hibiscus).
		Bold(true).
		Render("hibiscus")
	modelStyle := lipgloss.NewStyle().Foreground(theme.FGDim).Italic(true)
	leftHeader := fmt.Sprintf("%s %s", mark, wordmark)
	rightHeader := modelStyle.Render(fmt.Sprintf("○ %s", m.model))
	middle := strings.Repeat(" ", max(0, cw-lipgloss.Width(leftHeader)-lipgloss.Width(rightHeader)))
	header := lipgloss.NewStyle().
		Width(cw).
		Background(theme.BGBase).
		Render(leftHeader + middle + rightHeader)

	// ── Input area ────────────────────────────────────────────────
	beamColor := theme.Hibiscus
	if m.streaming && (m.animTick/4)%2 == 0 {
		beamColor = theme.HibiscusGlow
	}
	bgStyle := lipgloss.NewStyle().Background(theme.BGChat)
	beamStyled := bgStyle.Foreground(beamColor).Render("▌")
	gapStyled := bgStyle.Render(" ")

	inputW := cw - 2 // beam + gap
	if inputW < 10 {
		inputW = 10
	}

	var taLines []string
	if m.input.Value() == "" {
		phStyle := lipgloss.NewStyle().Background(theme.BGChat).Foreground(theme.FGDim)
		cursorStyle := lipgloss.NewStyle().Background(theme.FG).Foreground(theme.BGChat)
		phRunes := []rune(m.input.Placeholder)
		taLines = make([]string, m.input.Height())
		for i := 0; i < m.input.Height(); i++ {
			var content string
			if i == 0 && len(phRunes) > 0 {
				content = cursorStyle.Render(string(phRunes[0])) +
					phStyle.Render(string(phRunes[1:]))
			}
			pad := inputW - lipgloss.Width(content)
			if pad > 0 {
				content += bgStyle.Render(strings.Repeat(" ", pad))
			}
			taLines[i] = beamStyled + gapStyled + content
		}
	} else {
		rawLines := strings.Split(strings.TrimRight(m.input.View(), "\n"), "\n")
		taLines = make([]string, m.input.Height())
		for i := 0; i < m.input.Height(); i++ {
			var line string
			if i < len(rawLines) {
				line = rawLines[i]
			}
			pad := inputW - ansi.StringWidth(line)
			if pad > 0 {
				line += bgStyle.Render(strings.Repeat(" ", pad))
			}
			taLines[i] = beamStyled + gapStyled + line
		}
	}

	topPad := beamStyled + gapStyled + bgStyle.Render(strings.Repeat(" ", inputW))
	inputBlock := topPad + "\n" + strings.Join(taLines, "\n")

	// ── Status bar ────────────────────────────────────────────────
	var dot string
	if m.streaming {
		if (m.animTick/4)%2 == 0 {
			dot = lipgloss.NewStyle().Foreground(theme.HibiscusGlow).Render("●")
		} else {
			dot = lipgloss.NewStyle().Foreground(theme.Hibiscus).Render("●")
		}
	} else {
		dot = lipgloss.NewStyle().Foreground(theme.Hibiscus).Render("●")
	}
	providerLabel := config.ProviderLabel(m.cfg.LastProvider)
	statusLeft := fmt.Sprintf("%s %s · %s", dot, providerLabel, m.model)
	if m.activeSession != nil {
		sid := m.activeSession.SessionID
		if len(sid) > 8 {
			sid = sid[:8]
		}
		statusLeft += fmt.Sprintf(" · %s", sid)
	}

	var statusMid string
	if m.streaming {
		statusMid = ""
	} else if m.statusLine == "cancelled" {
		statusMid = lipgloss.NewStyle().Foreground(theme.Warning).Render("cancelled")
	} else if m.lastPromptTokens > 0 || m.lastCompletionTokens > 0 {
		statusMid = lipgloss.NewStyle().
			Foreground(theme.FGDim).
			Render(fmt.Sprintf("%d in / %d out", m.lastPromptTokens, m.lastCompletionTokens))
	}

	statusRight := lipgloss.NewStyle().
		Foreground(theme.HibiscusDim).
		Italic(true).
		Render(helpHint)

	gapTotal := cw - 2 - lipgloss.Width(statusLeft) - lipgloss.Width(statusMid) - lipgloss.Width(statusRight)
	gapTotal = max(0, gapTotal)
	leftGap := gapTotal / 2
	rightGap := gapTotal - leftGap
	statusBlock := lipgloss.NewStyle().
		Width(cw).
		Background(theme.BGBase).
		Foreground(theme.FG).
		Render(" " + statusLeft + strings.Repeat(" ", leftGap) + statusMid + strings.Repeat(" ", rightGap) + statusRight + " ")

	// Bottom padding — transparent so the terminal's default bg shows.
	bottomBlock := strings.Repeat(" ", cw)

	// Gap between input area and status bar — transparent so the
	// terminal's default background shows through.
	gapBlock := strings.Repeat(" ", cw)

	// Slash command autocomplete dropdown.
	dropdown := ""
	if m.cmdDropdown && len(m.cmdMatches) > 0 {
		dropdown = renderCmdDropdown(m.cmdMatches, m.cmdCursor, cw)
	}

	chatCol := lipgloss.JoinVertical(lipgloss.Left,
		header,
		m.viewport.View(),
		dropdown,
		inputBlock,
		gapBlock,
		statusBlock,
		bottomBlock,
	)

	// ── Right sidebar ─────────────────────────────────────────────
	if !sidebarVisible(m.width) {
		return chatCol
	}
	sidebar := m.renderSidebar()
	gap := lipgloss.NewStyle().Width(1).Background(theme.BGBase).Render(" ")
	return lipgloss.JoinHorizontal(lipgloss.Top, chatCol, gap, sidebar)
}

// workspaceSystemPrompt is the v0 system prompt. At v0 we use a minimal
// prompt that doesn't enable tools — full parity with cli/src/tools/
// definitions/prompt.rs comes in v0.1 when the agent loop lands.
func workspaceSystemPrompt() string {
	return strings.TrimSpace(`
You are Hibiscus, a terminal-based assistant. You are running inside
the new Go + Bubble Tea client (v0 — walking skeleton). Be concise and
helpful. Tool calls are not enabled at this version; if the user asks
for something that needs filesystem or shell access, explain what you
would do and suggest they run the Rust client (cli/) for now.
`)
}

// min returns the smaller of a or b. (Go 1.21+ has this as a builtin
// but kept here for explicitness at call sites.)
func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}

// isTerminalResponse returns true if s looks like a terminal control
// sequence (OSC color response, cursor position report, etc.) rather
// than genuine user input. These leak as KeyMsg events when the
// terminal responds to capability queries Bubble Tea sends at startup.
func isTerminalResponse(s string) bool {
	return strings.Contains(s, "rgb:") ||
		strings.Contains(s, "bg:") ||
		strings.Contains(s, "fg:") ||
		strings.Contains(s, "]11;") ||
		strings.Contains(s, "]10;") ||
		strings.ContainsRune(s, '\x1b')
}

// isOSCFragment returns true if s looks like a fragment of an OSC
// response that arrived as a separate key event — short strings of
// non-letter characters like "]", "\", ";", "11", "/", hex digits, etc.
// Letters (except hex a-f and the "rgb"/"bg" indicators) are excluded
// so normal typing isn't swallowed.
func isOSCFragment(s string) bool {
	if len(s) == 0 || len(s) > 30 {
		return false
	}
	for _, c := range s {
		switch {
		case c >= '0' && c <= '9':
		case c >= 'a' && c <= 'f': // hex digits
		case c == ']' || c == ';' || c == ':' || c == '/' || c == '\\':
		case c == 'b' || c == 'g' || c == 'r': // "rgb", "bg"
		default:
			return false
		}
	}
	return true
}

// looksLikeGarbage returns true if the textarea value is likely OSC
// response garbage rather than real user input. Used by the post-loading
// cleanup to catch fragments that slipped through the event filter.
// Returns false if the value contains any letter (a-z, A-Z) or space,
// since those indicate intentional typing.
func looksLikeGarbage(s string) bool {
	if len(s) == 0 || len(s) > 30 {
		return false
	}
	for _, c := range s {
		if (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == ' ' {
			return false
		}
	}
	return true
}
