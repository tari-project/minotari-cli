# Webhooks

Minotari provides a robust webhook system that allows your application to receive real-time notifications about wallet events. This is particularly useful when running the wallet in **Daemon Mode**, allowing you to build reactive applications (e.g., updating a user's balance when a deposit is confirmed) without polling the API.

## Configuration

Webhooks are configured via the `config.toml` file or through environment variables. You must provide both a target URL and a secret key to enable the feature.

### Using `config.toml`

Add the `[wallet.webhook]` section to your configuration file:

```toml
[wallet.webhook]
# The HTTP(S) endpoint where Minotari will send POST requests
url = "https://your-app.com/api/minotari-events"

# A secret string used to sign payloads (HMAC-SHA256)
# Generate a strong, random string for this.
secret = "whsec_..." 
```

### Using Environment Variables

Minotari supports configuration via environment variables, which is recommended for containerized deployments (Docker/Kubernetes).

*   **URL:** `TARI_WALLET__WEBHOOK__URL`
*   **Secret:** `TARI_WALLET__WEBHOOK__SECRET`

## Event Delivery

Webhooks are delivered asynchronously. The system includes a retry mechanism with exponential backoff for transient failures (e.g., network timeouts, HTTP 5xx responses).

*   **HTTP Method:** `POST`
*   **Content-Type:** `application/json`
*   **Timeout:** 20 seconds

## Payload Structure

Every webhook request contains a JSON body with a standard envelope structure.

```json
{
  "event_id": 12345,
  "event_type": "OutputDetected",
  "created_at": "2024-01-01T12:00:00+00:00",
  "balance": {
    "available": 5000000,
    "pending_incoming": 1000000,
    "pending_outgoing": 0
  },
  "data": {
    "OutputDetected": {
      "hash": "7b...3a",
      "block_height": 15000,
      "block_hash": [23, 12, ...],
      "memo_parsed": "Invoice #101",
      "memo_hex": "496e766f6963652023313031"
    }
  }
}
```

### Fields

*   **`event_id`** *(integer)*: A unique, incremental identifier for the event. Use this for idempotency (deduplication).
*   **`event_type`** *(string)*: The type of event (e.g., `OutputDetected`, `OutputConfirmed`, `TransactionConfirmed`, `BlockRolledBack`).
*   **`created_at`** *(string)*: ISO 8601 timestamp of when the event occurred.
*   **`balance`** *(object)*: A snapshot of the wallet balance at the exact moment the event was triggered.
    *   Values are in micro-minotari.
*   **`data`** *(object)*: Event-specific details. The key matches the `event_type`.

## Security & Verification

To ensure that requests received by your server actually come from your Minotari wallet instance, every request is signed using the configured `secret`.

### Headers

The following headers are added to every request:

*   **`X-Minotari-Signature`**: The HMAC signature (e.g., `t=1709294000,v1=6d3f...`).
*   **`X-Minotari-Timestamp`**: The Unix timestamp (seconds) when the request was signed.

### Verifying the Signature

The signature is constructed as an HMAC-SHA256 hash.
The string to sign is constructed by concatenating the timestamp, a period (`.`), and the raw JSON request body.

**Format:** `{timestamp}.{json_body}`

#### Step-by-Step Verification

1.  Extract the timestamp and signature hash from the `X-Minotari-Signature` header.
    *   The header format is `t={timestamp},v1={signature}`.
2.  Check if the timestamp is recent (e.g., within 5 minutes) to prevent replay attacks.
3.  Concatenate the timestamp (as a string), the character `.`, and the raw request body.
4.  Compute the HMAC-SHA256 of this string using your webhook secret.
5.  Compare your computed hash with the signature provided in the header using a constant-time comparison.

### Code Examples

#### Node.js (Express)

```javascript
const crypto = require('crypto');
const express = require('express');
const app = express();

// Ensure you use a body parser that gives you the raw buffer
app.use(express.json({
  verify: (req, res, buf) => {
    req.rawBody = buf;
  }
}));

const WEBHOOK_SECRET = process.env.WEBHOOK_SECRET;

app.post('/webhook', (req, res) => {
  const signatureHeader = req.headers['x-minotari-signature'];
  if (!signatureHeader) return res.status(400).send('Missing signature');

  // 1. Extract timestamp and signature
  const parts = signatureHeader.split(',');
  const timestamp = parts.find(p => p.startsWith('t=')).split('=')[1];
  const signature = parts.find(p => p.startsWith('v1=')).split('=')[1];

  // 2. Prevent Replay Attacks (e.g., 5 minute tolerance)
  const now = Math.floor(Date.now() / 1000);
  if (Math.abs(now - parseInt(timestamp)) > 300) {
    return res.status(400).send('Request timestamp too old');
  }

  // 3. Construct the string to sign
  const payload = `${timestamp}.${req.rawBody}`;

  // 4. Compute HMAC
  const hmac = crypto.createHmac('sha256', WEBHOOK_SECRET)
                     .update(payload)
                     .digest('hex');

  // 5. Compare signatures
  if (crypto.timingSafeEqual(Buffer.from(signature), Buffer.from(hmac))) {
    // Verified!
    console.log('Event received:', req.body.event_type);
    res.status(200).send('OK');
  } else {
    res.status(401).send('Invalid signature');
  }
});
```

#### Python (Flask)

```python
import hmac
import hashlib
import time
from flask import Flask, request, abort

app = Flask(__name__)
WEBHOOK_SECRET = "your_secret_string"

@app.route('/webhook', methods=['POST'])
def handle_webhook():
    sig_header = request.headers.get('X-Minotari-Signature')
    if not sig_header:
        abort(400, 'Missing signature header')

    # Parse header
    parts = {k: v for k, v in [p.split('=') for p in sig_header.split(',')]}
    timestamp = parts.get('t')
    signature = parts.get('v1')

    # Prevent Replay
    if abs(time.time() - int(timestamp)) > 300:
        abort(400, 'Request too old')

    # Construct payload
    payload = f"{timestamp}.{request.data.decode('utf-8')}"
    
    # Compute HMAC
    computed_sig = hmac.new(
        WEBHOOK_SECRET.encode('utf-8'),
        payload.encode('utf-8'),
        hashlib.sha256
    ).hexdigest()

    # Compare
    if hmac.compare_digest(computed_sig, signature):
        print(f"Verified event: {request.json.get('event_type')}")
        return "OK", 200
    else:
        abort(401, 'Invalid signature')
```

#### Go

```go
package main

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"net/http"
	"strconv"
	"strings"
	"time"
)

var secret = []byte("your_secret_string")

func webhookHandler(w http.ResponseWriter, r *http.Request) {
	sigHeader := r.Header.Get("X-Minotari-Signature")
	if sigHeader == "" {
		http.Error(w, "Missing signature", http.StatusBadRequest)
		return
	}

	body, err := io.ReadAll(r.Body)
	if err != nil {
		http.Error(w, "Error reading body", http.StatusInternalServerError)
		return
	}

	// Parse Header
	parts := strings.Split(sigHeader, ",")
	var timestamp, signature string
	for _, part := range parts {
		kv := strings.Split(part, "=")
		if kv[0] == "t" { timestamp = kv[1] }
		if kv[0] == "v1" { signature = kv[1] }
	}

	// Prevent Replay
	tsInt, _ := strconv.ParseInt(timestamp, 10, 64)
	if time.Now().Unix()-tsInt > 300 {
		http.Error(w, "Request too old", http.StatusBadRequest)
		return
	}

	// Verify
	payload := fmt.Sprintf("%s.%s", timestamp, string(body))
	mac := hmac.New(sha256.New, secret)
	mac.Write([]byte(payload))
	expectedSig := hex.EncodeToString(mac.Sum(nil))

	if hmac.Equal([]byte(signature), []byte(expectedSig)) {
		fmt.Println("Webhook verified!")
		w.WriteHeader(http.StatusOK)
	} else {
		http.Error(w, "Invalid signature", http.StatusUnauthorized)
	}
}
```
