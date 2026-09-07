package controlplane

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"io"
	"net/url"
	"regexp"
	"strconv"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"
)

const (
	MaxClientSyncBatchItems     = 100
	MaxClientSyncPageItems      = 200
	MaxClientSyncItemBytes      = 256 * 1024
	MaxClientSyncChunkTextBytes = 64 * 1024
	maxClientSyncStringBytes    = 4 * 1024
	maxClientSyncJSONDepth      = 8
	maxClientSyncJSONKeys       = 200
)

type ClientSyncKind string

const (
	ClientSyncKindPersonaVersion    ClientSyncKind = "persona_version"
	ClientSyncKindPolicyVersion     ClientSyncKind = "policy_version"
	ClientSyncKindModelConfig       ClientSyncKind = "model_config"
	ClientSyncKindKnowledgeSource   ClientSyncKind = "knowledge_source"
	ClientSyncKindKnowledgeDocument ClientSyncKind = "knowledge_document"
	ClientSyncKindKnowledgeChunk    ClientSyncKind = "knowledge_chunk"
	ClientSyncKindKnowledgeRule     ClientSyncKind = "knowledge_rule"
	ClientSyncKindMemory            ClientSyncKind = "memory"
)

type ClientSyncMutation struct {
	MutationID   string          `json:"mutation_id"`
	Kind         ClientSyncKind  `json:"kind"`
	ItemID       string          `json:"item_id"`
	BaseRevision int64           `json:"base_revision"`
	Deleted      bool            `json:"deleted"`
	Payload      json.RawMessage `json:"payload,omitempty"`
}

type ClientSyncItem struct {
	Kind              ClientSyncKind  `json:"kind"`
	ItemID            string          `json:"item_id"`
	Revision          int64           `json:"revision"`
	Deleted           bool            `json:"deleted"`
	Payload           json.RawMessage `json:"payload,omitempty"`
	UpdatedByDeviceID string          `json:"updated_by_device_id"`
	UpdatedAt         string          `json:"updated_at"`
}

type ClientSyncReceipt struct {
	MutationID string         `json:"mutation_id"`
	Kind       ClientSyncKind `json:"kind"`
	ItemID     string         `json:"item_id"`
	Revision   int64          `json:"revision"`
	Deleted    bool           `json:"deleted"`
}

type ClientSyncPage struct {
	Items          []ClientSyncItem `json:"items"`
	NextCursor     int64            `json:"next_cursor"`
	HasMore        bool             `json:"has_more"`
	ServerRevision int64            `json:"server_revision"`
}

type ClientSyncWriteResult struct {
	Items          []ClientSyncReceipt `json:"items"`
	ServerRevision int64               `json:"server_revision"`
}

var clientSyncIDPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`)
var clientSyncHashPattern = regexp.MustCompile(`^[0-9a-f]{64}$`)
var clientSyncPersonaIDPattern = regexp.MustCompile(`^global-persona-v([1-9][0-9]*)$`)
var clientSyncPolicyIDPattern = regexp.MustCompile(`^global-policy-v([1-9][0-9]*)$`)

func ValidClientSyncIdentifier(value string) bool {
	return clientSyncIDPattern.MatchString(value)
}

type clientSyncFieldKind uint8

const (
	clientSyncString clientSyncFieldKind = iota
	clientSyncID
	clientSyncInteger
	clientSyncNumber
	clientSyncBoolean
	clientSyncTimestamp
	clientSyncHash
	clientSyncObject
	clientSyncStringList
	clientSyncNullableString
	clientSyncNullableTimestamp
	clientSyncNullableInteger
	clientSyncNullableHash
	clientSyncNullableID
	clientSyncIDList
	clientSyncModelURL
)

type clientSyncFieldRule struct {
	kind     clientSyncFieldKind
	min      float64
	max      float64
	allowed  map[string]struct{}
	maxBytes int
	maxChars int
	maxItems int
}

var clientSyncSchemas = map[ClientSyncKind]map[string]clientSyncFieldRule{
	ClientSyncKindPersonaVersion: {
		"version": {kind: clientSyncInteger, min: 1}, "content": {kind: clientSyncObject}, "created_at": {kind: clientSyncTimestamp},
	},
	ClientSyncKindPolicyVersion: {
		"version": {kind: clientSyncInteger, min: 1}, "score_threshold": {kind: clientSyncNumber, min: 0, max: 1},
		"minimum_confidence": {kind: clientSyncNumber, min: 0, max: 1}, "daily_send_quota": {kind: clientSyncInteger, min: 0},
		"automation_level":  {kind: clientSyncString, allowed: stringSet("disabled", "dry_run", "manual", "limited", "regular")},
		"auto_send_enabled": {kind: clientSyncBoolean}, "content": {kind: clientSyncObject}, "created_at": {kind: clientSyncTimestamp},
	},
	ClientSyncKindModelConfig: {
		"retrieval_mode": {kind: clientSyncString, allowed: stringSet("economy", "high_quality")},
		"chat_base_url":  {kind: clientSyncModelURL}, "chat_model": {kind: clientSyncString},
		"embedding_base_url": {kind: clientSyncModelURL}, "embedding_model": {kind: clientSyncString},
		"embedding_dimensions": {kind: clientSyncNullableInteger, min: 1}, "config_version": {kind: clientSyncInteger, min: 1},
		"updated_at": {kind: clientSyncTimestamp},
	},
	ClientSyncKindKnowledgeSource: {
		"source_type": {kind: clientSyncString, allowed: stringSet("file", "manual", "web")}, "title": {kind: clientSyncString},
		"original_name": {kind: clientSyncString}, "source_key": {kind: clientSyncNullableString}, "content_hash": {kind: clientSyncNullableHash},
		"status":     {kind: clientSyncString, allowed: stringSet("active", "disabled", "failed")},
		"created_at": {kind: clientSyncTimestamp}, "updated_at": {kind: clientSyncTimestamp},
	},
	ClientSyncKindKnowledgeDocument: {
		"source_id": {kind: clientSyncID}, "version": {kind: clientSyncInteger, min: 1}, "content_hash": {kind: clientSyncHash},
		"parser_version": {kind: clientSyncString}, "chunker_version": {kind: clientSyncString},
		"status":   {kind: clientSyncString, allowed: stringSet("parsed", "indexing", "indexed", "failed")},
		"metadata": {kind: clientSyncObject}, "created_at": {kind: clientSyncTimestamp},
	},
	ClientSyncKindKnowledgeChunk: {
		"document_id": {kind: clientSyncID}, "ordinal": {kind: clientSyncInteger, min: 0},
		"text": {kind: clientSyncString, maxBytes: MaxClientSyncChunkTextBytes}, "locator": {kind: clientSyncObject},
		"chunker_version": {kind: clientSyncString}, "enabled": {kind: clientSyncBoolean}, "created_at": {kind: clientSyncTimestamp},
	},
	ClientSyncKindKnowledgeRule: {
		"document_id": {kind: clientSyncID}, "condition_kind": {kind: clientSyncString, allowed: stringSet("literal_contains", "semantic")},
		"condition_text": {kind: clientSyncString}, "reply_guidance": {kind: clientSyncString},
		"literal_terms": {kind: clientSyncStringList}, "record_ids": {kind: clientSyncIDList}, "rule_fingerprint": {kind: clientSyncHash},
		"status":     {kind: clientSyncString, allowed: stringSet("pending", "active", "rejected")},
		"created_at": {kind: clientSyncTimestamp}, "updated_at": {kind: clientSyncTimestamp},
	},
	ClientSyncKindMemory: {
		"reusable_situation": {kind: clientSyncString, maxBytes: 2000, maxChars: 500}, "response_pattern": {kind: clientSyncString, maxBytes: 2000, maxChars: 500},
		"tags": {kind: clientSyncStringList, maxChars: 128, maxItems: 20}, "exclusions": {kind: clientSyncStringList, maxChars: 128, maxItems: 20},
		"confidence": {kind: clientSyncNumber, min: 0, max: 1}, "revision": {kind: clientSyncInteger, min: 1},
		"expires_at": {kind: clientSyncNullableTimestamp}, "lifecycle_state": {kind: clientSyncString, allowed: stringSet("enabled", "disabled", "deleted")},
		"created_at": {kind: clientSyncTimestamp}, "updated_at": {kind: clientSyncTimestamp}, "cloud_source_ref": {kind: clientSyncID},
	},
}

func ValidateClientSyncMutations(mutations []ClientSyncMutation) error {
	if len(mutations) < 1 || len(mutations) > MaxClientSyncBatchItems {
		return ErrClientSyncSchemaInvalid
	}
	for _, mutation := range mutations {
		if err := validateClientSyncMutation(mutation); err != nil {
			return err
		}
	}
	return nil
}

func validateClientSyncMutation(mutation ClientSyncMutation) error {
	if !clientSyncIDPattern.MatchString(mutation.MutationID) || !clientSyncIDPattern.MatchString(mutation.ItemID) || mutation.BaseRevision < 0 {
		return ErrClientSyncSchemaInvalid
	}
	schema, ok := clientSyncSchemas[mutation.Kind]
	if !ok {
		return ErrClientSyncSchemaInvalid
	}
	if !validClientSyncSemanticID(mutation.Kind, mutation.ItemID, nil) {
		return ErrClientSyncSchemaInvalid
	}
	if mutation.Deleted {
		if len(bytes.TrimSpace(mutation.Payload)) != 0 && !bytes.Equal(bytes.TrimSpace(mutation.Payload), []byte("null")) {
			return ErrClientSyncSchemaInvalid
		}
		return validateClientSyncItemSize(mutation)
	}

	payload, err := decodeClientSyncPayload(mutation.Payload)
	if err != nil || containsClientSyncSecretKey(payload) || len(payload) != len(schema) {
		return ErrClientSyncSchemaInvalid
	}
	if !validClientSyncSemanticID(mutation.Kind, mutation.ItemID, payload) {
		return ErrClientSyncSchemaInvalid
	}
	for name, rule := range schema {
		value, exists := payload[name]
		if !exists || !validateClientSyncField(value, rule) {
			return ErrClientSyncSchemaInvalid
		}
	}
	return validateClientSyncItemSize(mutation)
}

func validClientSyncSemanticID(kind ClientSyncKind, itemID string, payload map[string]any) bool {
	var match []string
	switch kind {
	case ClientSyncKindModelConfig:
		return itemID == "global-model-config"
	case ClientSyncKindPersonaVersion:
		match = clientSyncPersonaIDPattern.FindStringSubmatch(itemID)
	case ClientSyncKindPolicyVersion:
		match = clientSyncPolicyIDPattern.FindStringSubmatch(itemID)
	default:
		return true
	}
	if len(match) != 2 {
		return false
	}
	if payload == nil {
		return true
	}
	version, ok := payload["version"].(json.Number)
	if !ok {
		return false
	}
	want, err := version.Int64()
	return err == nil && match[1] == strconv.FormatInt(want, 10)
}

func ClientSyncMutationHash(mutation ClientSyncMutation) (string, error) {
	if err := validateClientSyncMutation(mutation); err != nil {
		return "", err
	}
	var payload any
	if !mutation.Deleted {
		decoded, err := decodeClientSyncPayload(mutation.Payload)
		if err != nil {
			return "", ErrClientSyncSchemaInvalid
		}
		payload = decoded
	}
	canonical, err := json.Marshal(struct {
		MutationID   string         `json:"mutation_id"`
		Kind         ClientSyncKind `json:"kind"`
		ItemID       string         `json:"item_id"`
		BaseRevision int64          `json:"base_revision"`
		Deleted      bool           `json:"deleted"`
		Payload      any            `json:"payload,omitempty"`
	}{mutation.MutationID, mutation.Kind, mutation.ItemID, mutation.BaseRevision, mutation.Deleted, payload})
	if err != nil {
		return "", ErrClientSyncSchemaInvalid
	}
	digest := sha256.Sum256(canonical)
	return hex.EncodeToString(digest[:]), nil
}

func CanonicalClientSyncPayload(payload json.RawMessage) (json.RawMessage, error) {
	decoded, err := decodeClientSyncPayload(payload)
	if err != nil {
		return nil, ErrClientSyncSchemaInvalid
	}
	canonical, err := json.Marshal(decoded)
	if err != nil {
		return nil, ErrClientSyncSchemaInvalid
	}
	return canonical, nil
}

func decodeClientSyncPayload(raw json.RawMessage) (map[string]any, error) {
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.UseNumber()
	var payload map[string]any
	if err := decoder.Decode(&payload); err != nil || payload == nil {
		return nil, ErrClientSyncSchemaInvalid
	}
	var extra any
	if err := decoder.Decode(&extra); err != io.EOF {
		return nil, ErrClientSyncSchemaInvalid
	}
	return payload, nil
}

func validateClientSyncField(value any, rule clientSyncFieldRule) bool {
	switch rule.kind {
	case clientSyncString, clientSyncID, clientSyncTimestamp, clientSyncHash, clientSyncModelURL:
		text, ok := value.(string)
		if !ok || !validateClientSyncString(text, rule.maxBytes) || (rule.maxChars > 0 && utf8.RuneCountInString(text) > rule.maxChars) {
			return false
		}
		switch rule.kind {
		case clientSyncID:
			return clientSyncIDPattern.MatchString(text)
		case clientSyncTimestamp:
			return validClientSyncTimestamp(text)
		case clientSyncHash:
			return clientSyncHashPattern.MatchString(text)
		case clientSyncModelURL:
			return validClientSyncModelURL(text)
		default:
			_, restricted := rule.allowed[text]
			return len(rule.allowed) == 0 || restricted
		}
	case clientSyncNullableString:
		return value == nil || validateClientSyncField(value, clientSyncFieldRule{kind: clientSyncString, maxBytes: rule.maxBytes})
	case clientSyncNullableTimestamp:
		return value == nil || validateClientSyncField(value, clientSyncFieldRule{kind: clientSyncTimestamp})
	case clientSyncNullableHash:
		return value == nil || validateClientSyncField(value, clientSyncFieldRule{kind: clientSyncHash})
	case clientSyncNullableID:
		return value == nil || validateClientSyncField(value, clientSyncFieldRule{kind: clientSyncID})
	case clientSyncInteger, clientSyncNullableInteger:
		if value == nil && rule.kind == clientSyncNullableInteger {
			return true
		}
		number, ok := value.(json.Number)
		if !ok {
			return false
		}
		integer, err := number.Int64()
		return err == nil && float64(integer) >= rule.min && (rule.max == 0 || float64(integer) <= rule.max)
	case clientSyncNumber:
		number, ok := value.(json.Number)
		if !ok {
			return false
		}
		parsed, err := number.Float64()
		return err == nil && parsed >= rule.min && parsed <= rule.max
	case clientSyncBoolean:
		_, ok := value.(bool)
		return ok
	case clientSyncObject:
		object, ok := value.(map[string]any)
		return ok && validateClientSyncContainer(object, 1, new(int))
	case clientSyncStringList, clientSyncIDList:
		items, ok := value.([]any)
		maxItems := rule.maxItems
		if maxItems == 0 {
			maxItems = maxClientSyncJSONKeys
		}
		if !ok || len(items) > maxItems {
			return false
		}
		for _, item := range items {
			text, ok := item.(string)
			if !ok || !validateClientSyncString(text, rule.maxBytes) || (rule.maxChars > 0 && utf8.RuneCountInString(text) > rule.maxChars) || (rule.kind == clientSyncIDList && !clientSyncIDPattern.MatchString(text)) {
				return false
			}
		}
		return true
	default:
		return false
	}
}

func validateClientSyncContainer(value any, depth int, keys *int) bool {
	if depth > maxClientSyncJSONDepth {
		return false
	}
	switch typed := value.(type) {
	case map[string]any:
		*keys += len(typed)
		if *keys > maxClientSyncJSONKeys {
			return false
		}
		for key, nested := range typed {
			if !validateClientSyncString(key, 0) || !validateClientSyncContainer(nested, depth+1, keys) {
				return false
			}
		}
	case []any:
		if len(typed) > maxClientSyncJSONKeys {
			return false
		}
		for _, nested := range typed {
			if !validateClientSyncContainer(nested, depth+1, keys) {
				return false
			}
		}
	case string:
		return validateClientSyncString(typed, 0)
	case json.Number, bool, nil:
		return true
	default:
		return false
	}
	return true
}

func containsClientSyncSecretKey(value any) bool {
	switch typed := value.(type) {
	case map[string]any:
		for key, nested := range typed {
			normalized := normalizeClientSyncKey(key)
			for _, forbidden := range []string{"cookie", "password", "private", "apikey", "token", "ticket", "credentialref"} {
				if strings.Contains(normalized, forbidden) {
					return true
				}
			}
			if containsClientSyncSecretKey(nested) {
				return true
			}
		}
	case []any:
		for _, nested := range typed {
			if containsClientSyncSecretKey(nested) {
				return true
			}
		}
	}
	return false
}

func validateClientSyncString(value string, maxBytes int) bool {
	if maxBytes == 0 {
		maxBytes = maxClientSyncStringBytes
	}
	return len(value) <= maxBytes
}

func validClientSyncTimestamp(value string) bool {
	parsed, err := time.Parse(time.RFC3339Nano, value)
	return err == nil && parsed.Location() == time.UTC && strings.HasSuffix(value, "Z")
}

func validClientSyncModelURL(value string) bool {
	parsed, err := url.Parse(value)
	if err != nil || parsed.Hostname() == "" || parsed.User != nil || parsed.RawQuery != "" || parsed.Fragment != "" {
		return false
	}
	return parsed.Scheme == "http" || parsed.Scheme == "https"
}

func normalizeClientSyncKey(value string) string {
	var normalized strings.Builder
	for _, current := range value {
		if current >= 0xFF01 && current <= 0xFF5E {
			current -= 0xFEE0
		}
		if unicode.IsLetter(current) || unicode.IsNumber(current) {
			normalized.WriteRune(unicode.ToLower(current))
		}
	}
	return normalized.String()
}

func validateClientSyncItemSize(mutation ClientSyncMutation) error {
	payload, err := json.Marshal(mutation)
	if err != nil || len(payload) > MaxClientSyncItemBytes {
		return ErrClientSyncSchemaInvalid
	}
	return nil
}

func stringSet(values ...string) map[string]struct{} {
	result := make(map[string]struct{}, len(values))
	for _, value := range values {
		result[value] = struct{}{}
	}
	return result
}
