package dev.omalauncher

import dev.omalauncher.linux.AGENTS
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ChatTitleTest {
    private val claude = AGENTS.first { it.bin == "claude" }

    @Test fun agentWindowIsNamedAfterItsChat() {
        assertEquals("Pegar imágenes", chatTitle("✳ Pegar imágenes", "Claude Code", claude))
    }

    @Test fun spinnerDoesNotChangeIt() {
        // Claude Code swaps the leading glyph while it works.
        assertEquals(chatTitle("✳ Fix bug", "Claude Code", claude), chatTitle("⠂ Fix bug", "Claude Code", claude))
    }

    @Test fun claudeFromAPlainTerminalCounts() {
        assertEquals("Fix bug", chatTitle("✳ Fix bug", "Terminal", null))
    }

    @Test fun plainWindowsHaveNone() {
        assertNull(chatTitle("foot", "Terminal", null))
        assertNull(chatTitle("", "Terminal", null))
    }

    @Test fun agentWithoutAChatYetHasNone() {
        assertNull(chatTitle("✳ Claude Code", "Claude Code", claude))
    }
}
