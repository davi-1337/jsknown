package dev.jsknown.burp;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertNotNull;

final class PluginSmokeTest {
    @Test
    void extensionClassExists() {
        assertNotNull(new JsknownBurpExtension());
    }
}
