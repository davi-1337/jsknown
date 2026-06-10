package dev.jsknown.burp;

import burp.api.montoya.BurpExtension;
import burp.api.montoya.MontoyaApi;
import burp.api.montoya.http.handler.HttpHandler;
import burp.api.montoya.http.handler.HttpRequestToBeSent;
import burp.api.montoya.http.handler.HttpResponseReceived;
import burp.api.montoya.http.handler.RequestToBeSentAction;
import burp.api.montoya.http.handler.ResponseReceivedAction;
import burp.api.montoya.http.message.HttpRequestResponse;
import burp.api.montoya.http.message.responses.HttpResponse;

import javax.swing.JButton;
import javax.swing.JCheckBox;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.JTextField;
import java.awt.BorderLayout;
import java.awt.GridLayout;
import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse.BodyHandlers;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.LinkedHashMap;
import java.util.Map;

public final class JsknownBurpExtension implements BurpExtension, HttpHandler {
    private MontoyaApi api;
    private final HttpClient client = HttpClient.newBuilder()
        .connectTimeout(Duration.ofSeconds(2))
        .build();
    private final JTextField serverUrl = new JTextField("http://127.0.0.1:3333");
    private final JCheckBox enabled = new JCheckBox("Capture HTML and JavaScript responses", true);
    private final JCheckBox scopeOnly = new JCheckBox("Only send in-scope traffic", false);

    @Override
    public void initialize(MontoyaApi api) {
        this.api = api;
        api.extension().setName("jsknown");
        api.userInterface().registerSuiteTab("jsknown", settingsPanel());
        api.http().registerHttpHandler(this);
        api.logging().logToOutput("jsknown extension loaded");
    }

    @Override
    public RequestToBeSentAction handleHttpRequestToBeSent(HttpRequestToBeSent requestToBeSent) {
        return RequestToBeSentAction.continueWith(requestToBeSent);
    }

    @Override
    public ResponseReceivedAction handleHttpResponseReceived(HttpResponseReceived responseReceived) {
        if (!enabled.isSelected()) {
            return ResponseReceivedAction.continueWith(responseReceived);
        }

        HttpRequestResponse requestResponse = responseReceived.initiatingRequest().requestResponse();
        String url = requestResponse.request().url();
        if (scopeOnly.isSelected() && !api.scope().isInScope(url)) {
            return ResponseReceivedAction.continueWith(responseReceived);
        }

        HttpResponse response = responseReceived;
        String contentType = response.headerValue("Content-Type");
        if (!isInteresting(url, contentType, response.bodyToString())) {
            return ResponseReceivedAction.continueWith(responseReceived);
        }

        try {
            postIngest(requestResponse, response);
        } catch (Exception error) {
            api.logging().logToError("jsknown ingest failed: " + error.getMessage());
        }

        return ResponseReceivedAction.continueWith(responseReceived);
    }

    private JPanel settingsPanel() {
        JPanel panel = new JPanel(new BorderLayout(8, 8));
        JPanel fields = new JPanel(new GridLayout(0, 1, 4, 4));
        fields.add(new JLabel("jsknown server URL"));
        fields.add(serverUrl);
        fields.add(enabled);
        fields.add(scopeOnly);

        JButton test = new JButton("Test connection");
        test.addActionListener(event -> testConnection());
        panel.add(fields, BorderLayout.NORTH);
        panel.add(test, BorderLayout.SOUTH);
        return panel;
    }

    private void testConnection() {
        try {
            HttpRequest request = HttpRequest.newBuilder()
                .uri(URI.create(serverUrl.getText() + "/health"))
                .timeout(Duration.ofSeconds(3))
                .GET()
                .build();
            java.net.http.HttpResponse<String> response = client.send(request, BodyHandlers.ofString());
            api.logging().logToOutput("jsknown health: " + response.statusCode() + " " + response.body());
        } catch (Exception error) {
            api.logging().logToError("jsknown health check failed: " + error.getMessage());
        }
    }

    private void postIngest(HttpRequestResponse requestResponse, HttpResponse response) throws IOException, InterruptedException {
        String json = JsonPayload.from(requestResponse, response);
        HttpRequest request = HttpRequest.newBuilder()
            .uri(URI.create(serverUrl.getText() + "/ingest"))
            .timeout(Duration.ofSeconds(5))
            .header("Content-Type", "application/json")
            .POST(HttpRequest.BodyPublishers.ofString(json, StandardCharsets.UTF_8))
            .build();
        client.send(request, BodyHandlers.discarding());
    }

    private static boolean isInteresting(String url, String contentType, String body) {
        String lowerUrl = url.toLowerCase();
        String lowerType = contentType == null ? "" : contentType.toLowerCase();
        return lowerType.contains("html")
            || lowerType.contains("javascript")
            || lowerUrl.endsWith(".js")
            || lowerUrl.endsWith(".mjs")
            || body.trim().startsWith("<!doctype html")
            || body.contains("__webpack_require__")
            || body.contains("__vite__mapDeps");
    }

    private static final class JsonPayload {
        static String from(HttpRequestResponse requestResponse, HttpResponse response) {
            Map<String, String> requestHeaders = new LinkedHashMap<>();
            requestResponse.request().headers().forEach(header -> requestHeaders.put(header.name(), header.value()));
            Map<String, String> responseHeaders = new LinkedHashMap<>();
            response.headers().forEach(header -> responseHeaders.put(header.name(), header.value()));

            return "{"
                + "\"request\":{"
                + "\"method\":" + quote(requestResponse.request().method()) + ","
                + "\"url\":" + quote(requestResponse.request().url()) + ","
                + "\"headers\":" + map(requestHeaders)
                + "},"
                + "\"response\":{"
                + "\"status\":" + response.statusCode() + ","
                + "\"headers\":" + map(responseHeaders) + ","
                + "\"body\":" + quote(response.bodyToString())
                + "}"
                + "}";
        }

        private static String map(Map<String, String> values) {
            StringBuilder builder = new StringBuilder("{");
            boolean first = true;
            for (Map.Entry<String, String> entry : values.entrySet()) {
                if (!first) {
                    builder.append(',');
                }
                first = false;
                builder.append(quote(entry.getKey())).append(':').append(quote(entry.getValue()));
            }
            return builder.append('}').toString();
        }

        private static String quote(String value) {
            if (value == null) {
                return "null";
            }
            StringBuilder builder = new StringBuilder("\"");
            for (char ch : value.toCharArray()) {
                switch (ch) {
                    case '\\' -> builder.append("\\\\");
                    case '"' -> builder.append("\\\"");
                    case '\n' -> builder.append("\\n");
                    case '\r' -> builder.append("\\r");
                    case '\t' -> builder.append("\\t");
                    default -> {
                        if (ch < 0x20) {
                            builder.append(String.format("\\u%04x", (int) ch));
                        } else {
                            builder.append(ch);
                        }
                    }
                }
            }
            return builder.append('"').toString();
        }
    }
}
