package service

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"
	"time"

	"autoLive/backend/internal/controlplane"
)

func (s *ControlPlane) probeModelPoolAccount(ctx context.Context, account controlplane.ModelPoolAccountSummary, input controlplane.TestModelPoolAccountInput, secret string) controlplane.ModelPoolConnectivityTestResult {
	baseURL := strings.TrimRight(account.BaseURL, "/")
	if baseURL == "" {
		baseURL = "https://api.openai.com/v1"
	}
	startedAt := time.Now()
	result := controlplane.ModelPoolConnectivityTestResult{
		AccountID: account.ID,
		Provider:  account.Provider,
		Model:     account.Model,
		TestedAt:  s.repository.Now().Format(time.RFC3339),
	}
	endpoint, endpointErr := parseAndValidateModelPoolEndpoint(ctx, baseURL+"/models", s.modelPoolResolver, s.allowInsecureHTTP)
	if endpointErr != nil {
		result.Status = "failed"
		result.ErrorCode = modelPoolConnectivitySSRFBlocked
		if errors.Is(endpointErr, errModelPoolInsecureHTTP) {
			result.ErrorCode = modelPoolConnectivityInsecureHTTP
		}
	} else {
		client, clientErr := prepareModelPoolHTTPClient(s.httpClient, s.modelPoolResolver, s.allowInsecureHTTP)
		if clientErr != nil {
			result.Status = "failed"
			result.ErrorCode = modelPoolTransportUnsupported
		} else {
			request, requestErr := http.NewRequestWithContext(ctx, http.MethodGet, endpoint.String(), nil)
			if requestErr != nil {
				result.Status = "failed"
				result.ErrorCode = modelPoolConnectivitySSRFBlocked
			} else {
				request.Header.Set("Accept", "application/json")
				request.Header.Set("Authorization", "Bearer "+secret)
				// prepareModelPoolHTTPClient returns a per-request client clone;
				// keep the ControlPlane's shared base client immutable so concurrent
				// background probes cannot race on http.Client.Timeout.
				client.Timeout = time.Duration(input.TimeoutSeconds) * time.Second
				response, requestErr := client.Do(request)
				if requestErr != nil {
					result.Status = "failed"
					result.ErrorCode = modelPoolConnectivityFailed
					if ctx.Err() != nil || isModelPoolTimeout(requestErr) {
						result.Status = "timeout"
						result.ErrorCode = modelPoolConnectivityTimeout
					} else if errors.Is(requestErr, errModelPoolSSRFBlocked) {
						result.ErrorCode = modelPoolConnectivitySSRFBlocked
					} else if errors.Is(requestErr, errModelPoolInsecureHTTP) {
						result.ErrorCode = modelPoolConnectivityInsecureHTTP
					} else if errors.Is(requestErr, errModelPoolRedirectLimit) {
						result.ErrorCode = modelPoolConnectivityRedirectLimit
					}
				} else {
					defer response.Body.Close()
					result.HTTPStatus = response.StatusCode
					result.ResponseSummary = fmt.Sprintf("HTTP %d", response.StatusCode)
					if response.StatusCode < http.StatusOK || response.StatusCode >= http.StatusMultipleChoices {
						result.Status = "failed"
						result.ErrorCode = modelPoolProviderHTTPError
					} else {
						body, bodyErr := io.ReadAll(io.LimitReader(response.Body, modelPoolMaxResponseBytes+1))
						if bodyErr != nil {
							result.Status = "failed"
							result.ErrorCode = modelPoolConnectivityFailed
							if isModelPoolTimeout(bodyErr) {
								result.Status = "timeout"
								result.ErrorCode = modelPoolConnectivityTimeout
							}
						} else if len(body) > modelPoolMaxResponseBytes {
							result.Status = "failed"
							result.ErrorCode = modelPoolResponseTooLarge
						} else {
							found, decodeErr := modelPoolResponseContainsModel(body, account.Model)
							switch {
							case decodeErr != nil:
								result.Status = "failed"
								result.ErrorCode = modelPoolInvalidResponse
							case !found:
								result.Status = "failed"
								result.ErrorCode = modelPoolModelNotFound
							default:
								result.Status = "succeeded"
							}
						}
					}
				}
			}
		}
	}
	result.LatencyMS = time.Since(startedAt).Milliseconds()
	return result
}
