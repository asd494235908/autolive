package controlplane

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestValidateClientSyncMutationsAcceptsEveryAllowlistedKind(t *testing.T) {
	tests := []struct {
		kind    ClientSyncKind
		itemID  string
		payload string
	}{
		{ClientSyncKindPersonaVersion, "global-persona-v1", `{"version":1,"content":{"tone":"calm"},"created_at":"2026-09-05T01:02:03Z"}`},
		{ClientSyncKindPolicyVersion, "global-policy-v1", `{"version":1,"score_threshold":0.7,"minimum_confidence":0.6,"daily_send_quota":10,"automation_level":"manual","auto_send_enabled":false,"content":{"mode":"safe"},"created_at":"2026-09-05T01:02:03Z"}`},
		{ClientSyncKindModelConfig, "global-model-config", `{"retrieval_mode":"economy","chat_base_url":"https://api.example.test/v1","chat_model":"chat","embedding_base_url":"https://api.example.test/v1","embedding_model":"embed","embedding_dimensions":1024,"config_version":1,"updated_at":"2026-09-05T01:02:03Z"}`},
		{ClientSyncKindKnowledgeSource, "source-1", `{"source_type":"manual","title":"FAQ","original_name":"faq.txt","source_key":"faq","content_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","status":"active","created_at":"2026-09-05T01:02:03Z","updated_at":"2026-09-05T01:02:03Z"}`},
		{ClientSyncKindKnowledgeDocument, "document-1", `{"source_id":"source-1","version":1,"content_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","parser_version":"v1","chunker_version":"v1","status":"parsed","metadata":{"language":"zh-CN"},"created_at":"2026-09-05T01:02:03Z"}`},
		{ClientSyncKindKnowledgeChunk, "chunk-1", `{"document_id":"document-1","ordinal":0,"text":"hello","locator":{"page":1},"chunker_version":"v1","enabled":true,"created_at":"2026-09-05T01:02:03Z"}`},
		{ClientSyncKindKnowledgeRule, "rule-1", `{"document_id":"document-1","condition_kind":"literal_contains","condition_text":"price","reply_guidance":"answer from catalog","literal_terms":["price"],"record_ids":["record-1"],"rule_fingerprint":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","status":"active","created_at":"2026-09-05T01:02:03Z","updated_at":"2026-09-05T01:02:03Z"}`},
		{ClientSyncKindMemory, "memory-1", `{"reusable_situation":"customer asks price","response_pattern":"answer from catalog","tags":["sales"],"exclusions":[],"confidence":0.8,"revision":1,"expires_at":null,"lifecycle_state":"enabled","created_at":"2026-09-05T01:02:03Z","updated_at":"2026-09-05T01:02:03Z","cloud_source_ref":"source-1"}`},
	}

	for _, test := range tests {
		t.Run(string(test.kind), func(t *testing.T) {
			mutation := ClientSyncMutation{
				MutationID:   "mutation-1",
				Kind:         test.kind,
				ItemID:       test.itemID,
				BaseRevision: 0,
				Payload:      json.RawMessage(test.payload),
			}
			if err := ValidateClientSyncMutations([]ClientSyncMutation{mutation}); err != nil {
				t.Fatalf("ValidateClientSyncMutations() error = %v", err)
			}
		})
	}
}

func TestValidateClientSyncMutationsRejectsUnknownFieldsSecretsAndBounds(t *testing.T) {
	valid := ClientSyncMutation{
		MutationID: "mutation-1",
		Kind:       ClientSyncKindPersonaVersion,
		ItemID:     "global-persona-v1",
		Payload:    json.RawMessage(`{"version":1,"content":{"tone":"calm"},"created_at":"2026-09-05T01:02:03Z"}`),
	}
	tests := []struct {
		name      string
		mutations []ClientSyncMutation
	}{
		{name: "unknown kind", mutations: []ClientSyncMutation{{MutationID: "mutation-1", Kind: "other", ItemID: "item-1", Payload: valid.Payload}}},
		{name: "unknown payload field", mutations: []ClientSyncMutation{{MutationID: "mutation-1", Kind: valid.Kind, ItemID: "item-1", Payload: json.RawMessage(`{"version":1,"content":{},"created_at":"2026-09-05T01:02:03Z","extra":true}`)}}},
		{name: "nested secret key", mutations: []ClientSyncMutation{{MutationID: "mutation-1", Kind: valid.Kind, ItemID: "item-1", Payload: json.RawMessage(`{"version":1,"content":{"Private-Token":"secret"},"created_at":"2026-09-05T01:02:03Z"}`)}}},
		{name: "unsafe item id", mutations: []ClientSyncMutation{{MutationID: "mutation-1", Kind: valid.Kind, ItemID: "../item", Payload: valid.Payload}}},
		{name: "oversized chunk text", mutations: []ClientSyncMutation{{MutationID: "mutation-1", Kind: ClientSyncKindKnowledgeChunk, ItemID: "item-1", Payload: mustJSON(t, map[string]any{"document_id": "document-1", "ordinal": 0, "text": strings.Repeat("x", MaxClientSyncChunkTextBytes+1), "locator": map[string]any{}, "chunker_version": "v1", "enabled": true, "created_at": "2026-09-05T01:02:03Z"})}}},
		{name: "payload too deep", mutations: []ClientSyncMutation{{MutationID: "mutation-1", Kind: valid.Kind, ItemID: "item-1", Payload: json.RawMessage(`{"version":1,"content":{"a":{"b":{"c":{"d":{"e":{"f":{"g":{"h":{"i":1}}}}}}}}},"created_at":"2026-09-05T01:02:03Z"}`)}}},
		{name: "too many mutations", mutations: repeatMutation(valid, MaxClientSyncBatchItems+1)},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if err := ValidateClientSyncMutations(test.mutations); !IsErrorCode(err, ErrClientSyncSchemaInvalid.Code) {
				t.Fatalf("ValidateClientSyncMutations() error = %v, want %s", err, ErrClientSyncSchemaInvalid.Code)
			}
		})
	}
}

func TestValidateClientSyncMutationRequiresEmptyPayloadForTombstone(t *testing.T) {
	mutation := ClientSyncMutation{
		MutationID: "mutation-1",
		Kind:       ClientSyncKindPersonaVersion,
		ItemID:     "global-persona-v1",
		Deleted:    true,
		Payload:    json.RawMessage(`{"version":1}`),
	}
	if err := ValidateClientSyncMutations([]ClientSyncMutation{mutation}); !IsErrorCode(err, ErrClientSyncSchemaInvalid.Code) {
		t.Fatalf("ValidateClientSyncMutations() error = %v, want %s", err, ErrClientSyncSchemaInvalid.Code)
	}
	mutation.Payload = nil
	if err := ValidateClientSyncMutations([]ClientSyncMutation{mutation}); err != nil {
		t.Fatalf("tombstone validation error = %v", err)
	}
}

func TestValidateClientSyncMutationsEnforcesDesktopSemanticIDs(t *testing.T) {
	tests := []ClientSyncMutation{
		{MutationID: "mutation-1", Kind: ClientSyncKindModelConfig, ItemID: "model-1", Payload: json.RawMessage(`{"retrieval_mode":"economy","chat_base_url":"https://api.example.test/v1","chat_model":"chat","embedding_base_url":"https://api.example.test/v1","embedding_model":"embed","embedding_dimensions":1024,"config_version":1,"updated_at":"2026-09-05T01:02:03Z"}`)},
		{MutationID: "mutation-2", Kind: ClientSyncKindPersonaVersion, ItemID: "global-persona-v2", Payload: json.RawMessage(`{"version":1,"content":{},"created_at":"2026-09-05T01:02:03Z"}`)},
		{MutationID: "mutation-3", Kind: ClientSyncKindPolicyVersion, ItemID: "policy-1", Deleted: true},
	}
	for _, mutation := range tests {
		if err := ValidateClientSyncMutations([]ClientSyncMutation{mutation}); !IsErrorCode(err, ErrClientSyncSchemaInvalid.Code) {
			t.Fatalf("ValidateClientSyncMutations(%s/%s) error = %v, want schema invalid", mutation.Kind, mutation.ItemID, err)
		}
	}
}

func TestValidateClientSyncMutationsEnforcesDesktopMemoryListLimits(t *testing.T) {
	base := map[string]any{
		"reusable_situation": "customer asks price", "response_pattern": "answer from catalog",
		"tags": []string{"sales"}, "exclusions": []string{}, "confidence": 0.8,
		"revision": 1, "expires_at": nil, "lifecycle_state": "enabled",
		"created_at": "2026-09-05T01:02:03Z", "updated_at": "2026-09-05T01:02:03Z",
		"cloud_source_ref": "source-1",
	}
	tests := []struct {
		name  string
		field string
		value any
	}{
		{name: "too many tags", field: "tags", value: make([]string, 21)},
		{name: "oversized tag", field: "tags", value: []string{strings.Repeat("字", 129)}},
		{name: "too many exclusions", field: "exclusions", value: make([]string, 21)},
		{name: "oversized exclusion", field: "exclusions", value: []string{strings.Repeat("字", 129)}},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			payload := make(map[string]any, len(base))
			for key, value := range base {
				payload[key] = value
			}
			payload[test.field] = test.value
			mutation := ClientSyncMutation{MutationID: "mutation-1", Kind: ClientSyncKindMemory, ItemID: "memory-1", Payload: mustJSON(t, payload)}
			if err := ValidateClientSyncMutations([]ClientSyncMutation{mutation}); !IsErrorCode(err, ErrClientSyncSchemaInvalid.Code) {
				t.Fatalf("ValidateClientSyncMutations() error = %v, want schema invalid", err)
			}
		})
	}
	for _, field := range []string{"reusable_situation", "response_pattern"} {
		payload := make(map[string]any, len(base))
		for key, value := range base {
			payload[key] = value
		}
		payload[field] = strings.Repeat("x", 501)
		mutation := ClientSyncMutation{MutationID: "mutation-1", Kind: ClientSyncKindMemory, ItemID: "memory-1", Payload: mustJSON(t, payload)}
		if err := ValidateClientSyncMutations([]ClientSyncMutation{mutation}); !IsErrorCode(err, ErrClientSyncSchemaInvalid.Code) {
			t.Fatalf("ValidateClientSyncMutations(%s) error = %v, want schema invalid", field, err)
		}
	}
}

func TestClientSyncMutationHashCanonicalizesPayloadObjectOrder(t *testing.T) {
	left := ClientSyncMutation{MutationID: "mutation-1", Kind: ClientSyncKindPersonaVersion, ItemID: "global-persona-v1", Payload: json.RawMessage(`{"version":1,"content":{"tone":"calm"},"created_at":"2026-09-05T01:02:03Z"}`)}
	right := left
	right.Payload = json.RawMessage(`{"created_at":"2026-09-05T01:02:03Z","content":{"tone":"calm"},"version":1}`)
	leftHash, err := ClientSyncMutationHash(left)
	if err != nil {
		t.Fatalf("left hash error = %v", err)
	}
	rightHash, err := ClientSyncMutationHash(right)
	if err != nil {
		t.Fatalf("right hash error = %v", err)
	}
	if leftHash != rightHash || len(leftHash) != 64 {
		t.Fatalf("hashes = (%q, %q), want equal lowercase SHA-256", leftHash, rightHash)
	}
}

func TestValidateClientSyncMutationsRejectsMalformedHashesReferencesAndTrailingJSON(t *testing.T) {
	tests := []struct {
		name     string
		mutation ClientSyncMutation
	}{
		{name: "malformed nullable hash", mutation: ClientSyncMutation{MutationID: "mutation-1", Kind: ClientSyncKindKnowledgeSource, ItemID: "source-1", Payload: json.RawMessage(`{"source_type":"manual","title":"FAQ","original_name":"faq.txt","source_key":null,"content_hash":"not-a-hash","status":"active","created_at":"2026-09-05T01:02:03Z","updated_at":"2026-09-05T01:02:03Z"}`)}},
		{name: "unsafe nullable id", mutation: ClientSyncMutation{MutationID: "mutation-1", Kind: ClientSyncKindMemory, ItemID: "memory-1", Payload: json.RawMessage(`{"reusable_situation":"customer asks price","response_pattern":"answer from catalog","tags":[],"exclusions":[],"confidence":0.8,"revision":1,"expires_at":null,"lifecycle_state":"enabled","created_at":"2026-09-05T01:02:03Z","updated_at":"2026-09-05T01:02:03Z","cloud_source_ref":"../local"}`)}},
		{name: "trailing JSON", mutation: ClientSyncMutation{MutationID: "mutation-1", Kind: ClientSyncKindPersonaVersion, ItemID: "persona-1", Payload: json.RawMessage(`{"version":1,"content":{},"created_at":"2026-09-05T01:02:03Z"} trailing`)}},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if err := ValidateClientSyncMutations([]ClientSyncMutation{test.mutation}); !IsErrorCode(err, ErrClientSyncSchemaInvalid.Code) {
				t.Fatalf("ValidateClientSyncMutations(%s) error = %v, want schema invalid", test.mutation.Kind, err)
			}
		})
	}
}

func TestValidateClientSyncMutationsRejectsUnsafeModelURLsAndNonPortableReferences(t *testing.T) {
	validModel := `{"retrieval_mode":"economy","chat_base_url":"https://api.example.test/v1","chat_model":"chat","embedding_base_url":"https://api.example.test/v1","embedding_model":"embed","embedding_dimensions":1024,"config_version":1,"updated_at":"2026-09-05T01:02:03Z"}`
	tests := []struct {
		name    string
		kind    ClientSyncKind
		payload string
	}{
		{name: "URL credentials", kind: ClientSyncKindModelConfig, payload: strings.Replace(validModel, "https://api.example.test/v1", "https://user:password@api.example.test/v1", 1)},
		{name: "URL secret query", kind: ClientSyncKindModelConfig, payload: strings.Replace(validModel, "https://api.example.test/v1", "https://api.example.test/v1?token=secret", 1)},
		{name: "missing memory source", kind: ClientSyncKindMemory, payload: `{"reusable_situation":"customer asks price","response_pattern":"answer from catalog","tags":[],"exclusions":[],"confidence":0.8,"revision":1,"expires_at":null,"lifecycle_state":"enabled","created_at":"2026-09-05T01:02:03Z","updated_at":"2026-09-05T01:02:03Z","cloud_source_ref":null}`},
		{name: "unsafe rule record ID", kind: ClientSyncKindKnowledgeRule, payload: `{"document_id":"document-1","condition_kind":"literal_contains","condition_text":"price","reply_guidance":"answer from catalog","literal_terms":["price"],"record_ids":["../record"],"rule_fingerprint":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","status":"active","created_at":"2026-09-05T01:02:03Z","updated_at":"2026-09-05T01:02:03Z"}`},
		{name: "spaced secret key", kind: ClientSyncKindPersonaVersion, payload: `{"version":1,"content":{"api key":"secret"},"created_at":"2026-09-05T01:02:03Z"}`},
		{name: "fullwidth secret key", kind: ClientSyncKindPersonaVersion, payload: `{"version":1,"content":{"ｔｏｋｅｎ":"secret"},"created_at":"2026-09-05T01:02:03Z"}`},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			mutation := ClientSyncMutation{MutationID: "mutation-1", Kind: test.kind, ItemID: "item-1", Payload: json.RawMessage(test.payload)}
			if err := ValidateClientSyncMutations([]ClientSyncMutation{mutation}); !IsErrorCode(err, ErrClientSyncSchemaInvalid.Code) {
				t.Fatalf("ValidateClientSyncMutations() error = %v, want schema invalid", err)
			}
		})
	}
}

func TestValidateClientSyncMutationsAllowsModelHTTP(t *testing.T) {
	mutation := ClientSyncMutation{
		MutationID: "mutation-1", Kind: ClientSyncKindModelConfig, ItemID: "global-model-config",
		Payload: json.RawMessage(`{"retrieval_mode":"economy","chat_base_url":"http://127.0.0.1:11434/v1","chat_model":"chat","embedding_base_url":"http://localhost:11434/v1","embedding_model":"embed","embedding_dimensions":1024,"config_version":1,"updated_at":"2026-09-05T01:02:03Z"}`),
	}
	if err := ValidateClientSyncMutations([]ClientSyncMutation{mutation}); err != nil {
		t.Fatalf("ValidateClientSyncMutations() error = %v", err)
	}
}

func TestValidateClientSyncMutationsAllowsTrustedProviderModelHTTP(t *testing.T) {
	mutation := ClientSyncMutation{
		MutationID: "mutation-1", Kind: ClientSyncKindModelConfig, ItemID: "global-model-config",
		Payload: json.RawMessage(`{"retrieval_mode":"economy","chat_base_url":"http://101.96.208.132:8081/v1","chat_model":"chat","embedding_base_url":"http://10.0.0.8:8081/v1","embedding_model":"embed","embedding_dimensions":1024,"config_version":1,"updated_at":"2026-09-05T01:02:03Z"}`),
	}
	if err := ValidateClientSyncMutations([]ClientSyncMutation{mutation}); err != nil {
		t.Fatalf("ValidateClientSyncMutations() error = %v", err)
	}
}

func TestCanonicalClientSyncPayloadRejectsTrailingInvalidJSON(t *testing.T) {
	if _, err := CanonicalClientSyncPayload(json.RawMessage(`{"version":1} trailing`)); !IsErrorCode(err, ErrClientSyncSchemaInvalid.Code) {
		t.Fatalf("CanonicalClientSyncPayload() error = %v, want schema invalid", err)
	}
}

func mustJSON(t *testing.T, value any) json.RawMessage {
	t.Helper()
	payload, err := json.Marshal(value)
	if err != nil {
		t.Fatalf("json.Marshal() error = %v", err)
	}
	return payload
}

func repeatMutation(mutation ClientSyncMutation, count int) []ClientSyncMutation {
	items := make([]ClientSyncMutation, count)
	for index := range items {
		items[index] = mutation
		items[index].MutationID = "mutation-" + strings.Repeat("x", index%2)
	}
	return items
}
