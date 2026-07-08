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

	"github.com/charmbracelet/bubbles/textarea"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/charmbracelet/x/ansi"

	"github.com/hmanlab/hmanlab/tui/internal/config"
	"github.com/hmanlab/hmanlab/tui/internal/llm"
	"github.com/hmanlab/hmanlab/tui/internal/ui/theme"
)

// Mode tracks which top-level view is on screen. Mirrors cli's
// Mode enum (cli/src/app/state.rs:196) but only the v0 subset.
type Mode int

const (
	ModeConnect Mode = iota
	ModeChat
)

// Model is the top-level Bubble Tea state. Everything the TUI knows
// lives on this struct — Update is pure dispatch, View is pure render.
type Model struct {
	mode Mode

	cfg      *config.Config
	provider llm.Provider
	model    string

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
// passes in a loaded config; if the config has no Ollama Cloud key,
// we boot straight into ModeConnect for the first-run flow.
func New(cfg *config.Config) Model {
	ta := textarea.New()
	ta.Placeholder = "Send a message…"
	ta.Prompt = ""
	ta.ShowLineNumbers = false
	ta.CharLimit = 0
	ta.SetHeight(3)
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
	m := Model{
		cfg:        cfg,
		input:      ta,
		viewport:   vp,
		followTail: true,
	}

	if cfg.HasOllamaCloudKey() {
		m.bootIntoChat()
	} else {
		m.mode = ModeConnect
		m.connect = newConnectModal()
	}
	return m
}

// bootIntoChat wires up the provider + model from config and switches
// to ModeChat. Used both at startup (when a key is already configured)
// and after the connect modal successfully validates a freshly entered key.
func (m *Model) bootIntoChat() {
	m.provider = llm.NewCloudOllama(m.cfg.OllamaCloudAPIKey)
	m.model = m.cfg.EffectiveModel()
	m.mode = ModeChat
	m.statusLine = "ready"
	m.history = []chatMessage{
		infoLine(fmt.Sprintf("Connected to **Ollama Cloud** (%s). Try /help for commands.", m.model)),
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
	switch v := msg.(type) {
	case tea.WindowSizeMsg:
		m.width, m.height = int(v.Width), int(v.Height)
		m.relayout()
		return m, nil

	case tickMsg:
		m.animTick++
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
		// Mark the trailing message as no longer streaming.
		for i := len(m.history) - 1; i >= 0; i-- {
			if m.history[i].streaming {
				m.history[i].streaming = false
				if v.cancelled {
					m.history[i].cancelled = true
				}
				break
			}
		}
		m.lastPromptTokens = v.promptTokens
		m.lastCompletionTokens = v.completionTokens
		m.statusLine = ""
		m.refreshViewportContent()
		return m, nil
	}

	// Mode-specific routing.
	switch m.mode {
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
	switch k := msg.(type) {
	case tea.KeyMsg:
		// Filter terminal OSC responses (e.g. background color queries)
		// that leak as key events — prevents garbage like "1;rgb:..."
		// from landing in the textarea.
		if strings.Contains(k.String(), "rgb:") {
			return m, nil
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
	m.relayout() // grow/shrink textarea with content
	return m, tea.Batch(cmd1, cmd2)
}

// sendUserMessage appends a user turn, opens a streaming request, and
// returns the cmds needed to start polling the stream.
func (m Model) sendUserMessage(text string) (tea.Model, tea.Cmd) {
	m.history = append(m.history, chatMessage{
		role:    llm.RoleUser,
		content: text,
	})
	m.input.Reset()

	// Build the message slice we send to the provider — full history,
	// excluding the leading info line and any cancelled/empty messages.
	apiMsgs := []llm.Message{
		{Role: llm.RoleSystem, Content: workspaceSystemPrompt()},
	}
	for _, h := range m.history {
		if h.role == llm.RoleSystem {
			continue
		}
		if h.content == "" {
			continue
		}
		apiMsgs = append(apiMsgs, llm.Message{Role: h.role, Content: h.content})
	}

	// Open the stream with a cancellable context.
	ctx, cancel := context.WithCancel(context.Background())
	m.streamCtx, m.streamCancel = ctx, cancel
	m.streamCh = m.provider.StreamChat(ctx, m.model, apiMsgs)
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
			m.history = append(m.history, infoLine(fmt.Sprintf("Current model: %s", m.model)))
			m.input.Reset()
			m.refreshViewportContent()
			return m, nil
		}
		// Validate against the Ollama Cloud catalog.
		valid := false
		for _, name := range m.provider.Models() {
			if name == arg {
				valid = true
				break
			}
		}
		if !valid {
			m.history = append(m.history, infoLine(fmt.Sprintf(
				"✕ unknown model %q. Valid: %s",
				arg, strings.Join(m.provider.Models()[:min(8, len(m.provider.Models()))], ", ")+", …",
			)))
		} else {
			m.model = arg
			m.cfg.LastModel = arg
			_ = m.cfg.Save()
			m.history = append(m.history, infoLine(fmt.Sprintf("✓ switched to %s", arg)))
		}
		m.input.Reset()
		m.followTail = true
		m.refreshViewportContent()
		return m, nil
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
// sees the response was truncated rather than complete.
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
			break
		}
	}
	m.statusLine = "cancelled"
	m.refreshViewportContent()
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
	const (
		headerH  = 1
		statusH  = 1
		bottomH  = 1 // breathing room under status bar
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
	vpH := m.height - headerH - inputH - statusH - bottomH
	if vpH < 3 {
		vpH = 3
	}
	if m.mode == ModeChat {
		m.viewport.Width = m.width
		m.viewport.Height = vpH
		m.input.SetHeight(taLines)
		// Textarea width = total - 1 (slim beam) - 1 (gap after beam).
		inputW := m.width - 1 - 1
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
	bodyW := m.width - 4 // gutter + breathing room
	if bodyW < 10 {
		bodyW = 10
	}
	var blocks []string
	for _, h := range m.history {
		blocks = append(blocks, h.render(bodyW, m.width, m.animTick))
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
	case ModeConnect:
		return m.connect.View(m.width, m.height)
	case ModeChat:
		return m.viewChat()
	}
	return ""
}

// viewChat renders the single-pane chat layout. Vertical stack
// (top → bottom): hibiscus header strip · history viewport · input
// area with thick hibiscus left beam · status bar (model name on the
// left, /help hint on the right, live indicator in the middle).
func (m Model) viewChat() string {
	// ── Header strip ──────────────────────────────────────────────
	// Hibiscus flower mark + wordmark on the left, model name on the
	// right. Both tinted hibiscus so the brand reads at first glance.
	mark := lipgloss.NewStyle().Foreground(theme.HibiscusGlow).Render("✿")
	wordmark := lipgloss.NewStyle().
		Foreground(theme.Hibiscus).
		Bold(true).
		Render("hibiscus")
	modelStyle := lipgloss.NewStyle().Foreground(theme.FGDim).Italic(true)
	leftHeader := fmt.Sprintf("%s %s", mark, wordmark)
	rightHeader := modelStyle.Render(fmt.Sprintf("○ %s", m.model))
	middle := strings.Repeat(" ", max(0, m.width-lipgloss.Width(leftHeader)-lipgloss.Width(rightHeader)))
	header := lipgloss.NewStyle().
		Width(m.width).
		Background(theme.BGBase).
		Render(leftHeader + middle + rightHeader)

	// ── Input area ────────────────────────────────────────────────
	// No full border — just a slim 1-col hibiscus left beam, then 1
	// col of breathing room, then the textarea. The `▌` glyph fills
	// only the left half of its cell, so the visible beam reads as a
	// thin pink line — not the chunky 2-col block we had before. While
	// a response streams, the beam pulses between Hibiscus and
	// HibiscusGlow on the same anim-tick cadence as the status-bar
	// dot so the two pulse in sync.
	beamColor := theme.Hibiscus
	if m.streaming && (m.animTick/4)%2 == 0 {
		beamColor = theme.HibiscusGlow
	}
	bgStyle := lipgloss.NewStyle().Background(theme.BGChat)
	beamStyled := bgStyle.Foreground(beamColor).Render("▌")
	gapStyled := bgStyle.Render(" ")

	inputW := m.width - 2 // beam + gap
	if inputW < 10 {
		inputW = 10
	}

	var taLines []string
	if m.input.Value() == "" {
		// bubbles/textarea's placeholderView emits unstyled trailing
		// spaces — render the placeholder manually with full BGChat.
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

	// 1 row of top padding for breathing room above the textarea.
	topPad := beamStyled + gapStyled + bgStyle.Render(strings.Repeat(" ", inputW))
	inputBlock := topPad + "\n" + strings.Join(taLines, "\n")

	// ── Status bar ────────────────────────────────────────────────
	// Three sections, left → right: connection dot + provider + model
	// · live indicator (streaming pulse or last-turn tokens) · /help
	// hint. The hint lives here now (same row as the model name) so
	// the input area can stay visually clean — just the beam + text.
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
	statusLeft := fmt.Sprintf("%s ollama-cloud · %s", dot, m.model)

	var statusMid string
	if m.streaming {
		statusMid = "" // generating indicator is in the chat header now
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

	// Pad between the three sections so left→mid→right layout holds at
	// any width. Two gaps: left↔mid and mid↔right.
	gapTotal := m.width - 2 - lipgloss.Width(statusLeft) - lipgloss.Width(statusMid) - lipgloss.Width(statusRight)
	gapTotal = max(0, gapTotal)
	leftGap := gapTotal / 2
	rightGap := gapTotal - leftGap
	statusBlock := lipgloss.NewStyle().
		Width(m.width).
		Background(theme.BGBase).
		Foreground(theme.FG).
		Render(" " + statusLeft + strings.Repeat(" ", leftGap) + statusMid + strings.Repeat(" ", rightGap) + statusRight + " ")

	// Bottom padding row — full-width blank BGBase strip one row tall.
	// Without this the status bar's footer text sits on the terminal's
	// very last row, reading as crammed. One row of BGBase lifts the
	// whole footer up visually and ends the layout on a clean break.
	bottomBlock := lipgloss.NewStyle().
		Width(m.width).
		Background(theme.BGBase).
		Render(strings.Repeat(" ", m.width))

	return lipgloss.JoinVertical(lipgloss.Left,
		header,
		m.viewport.View(),
		inputBlock,
		statusBlock,
		bottomBlock,
	)
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
