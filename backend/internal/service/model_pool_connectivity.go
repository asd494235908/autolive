package service

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"strconv"
	"strings"
	"time"
)

const (
	modelPoolMaxRedirects              = 3
	modelPoolMaxResponseHeaderBytes    = 64 << 10
	modelPoolMaxResponseBytes          = 1 << 20
	modelPoolConnectivityFailed        = "MODEL_POOL_CONNECTIVITY_FAILED"
	modelPoolConnectivityTimeout       = "MODEL_POOL_CONNECTIVITY_TIMEOUT"
	modelPoolConnectivitySSRFBlocked   = "MODEL_POOL_CONNECTIVITY_SSRF_BLOCKED"
	modelPoolConnectivityInsecureHTTP  = "MODEL_POOL_CONNECTIVITY_INSECURE_HTTP"
	modelPoolConnectivityRedirectLimit = "MODEL_POOL_CONNECTIVITY_REDIRECT_LIMIT"
	modelPoolResponseTooLarge          = "MODEL_POOL_RESPONSE_TOO_LARGE"
	modelPoolInvalidResponse           = "MODEL_POOL_INVALID_RESPONSE"
	modelPoolModelNotFound             = "MODEL_POOL_MODEL_NOT_FOUND"
	modelPoolProviderHTTPError         = "MODEL_POOL_PROVIDER_HTTP_ERROR"
	modelPoolTransportUnsupported      = "MODEL_POOL_CONNECTIVITY_TRANSPORT_UNSUPPORTED"
)

var (
	errModelPoolSSRFBlocked   = errors.New("model pool connectivity target is not allowed")
	errModelPoolInsecureHTTP  = errors.New("model pool connectivity target requires explicit insecure HTTP opt-in")
	errModelPoolRedirectLimit = errors.New("model pool connectivity redirect limit exceeded")
	errModelPoolTransport     = errors.New("model pool connectivity transport is not supported")
	modelPoolBlockedPrefixes  = mustModelPoolPrefixes([]string{
		"0.0.0.0/8",
		"100.64.0.0/10",
		"169.254.0.0/16",
		"192.0.0.0/24",
		"198.18.0.0/15",
		"224.0.0.0/4",
		"240.0.0.0/4",
		"::/128",
		"ff00::/8",
		"fe80::/10",
	})
	modelPoolMetadataAddresses = map[netip.Addr]struct{}{
		netip.MustParseAddr("100.100.100.200"): {},
		netip.MustParseAddr("168.63.129.16"):   {},
		netip.MustParseAddr("169.254.169.253"): {},
		netip.MustParseAddr("169.254.169.254"): {},
	}
)

type modelPoolIPResolver func(context.Context, string) ([]netip.Addr, error)

func defaultModelPoolIPResolver(ctx context.Context, host string) ([]netip.Addr, error) {
	return net.DefaultResolver.LookupNetIP(ctx, "ip", host)
}

func mustModelPoolPrefixes(values []string) []netip.Prefix {
	prefixes := make([]netip.Prefix, 0, len(values))
	for _, value := range values {
		prefixes = append(prefixes, netip.MustParsePrefix(value))
	}
	return prefixes
}

func isDisallowedModelPoolIP(address netip.Addr) bool {
	address = address.Unmap()
	if address.IsLoopback() || address.IsPrivate() || address.IsLinkLocalUnicast() || address.IsLinkLocalMulticast() || address.IsMulticast() || address.IsUnspecified() {
		return true
	}
	if _, exists := modelPoolMetadataAddresses[address]; exists {
		return true
	}
	for _, prefix := range modelPoolBlockedPrefixes {
		if prefix.Contains(address) {
			return true
		}
	}
	return false
}

func modelPoolEndpointPort(parsed *url.URL) (int, error) {
	portText := parsed.Port()
	if portText == "" {
		if strings.HasSuffix(parsed.Host, ":") {
			return 0, errModelPoolSSRFBlocked
		}
		if parsed.Scheme == "http" {
			return 80, nil
		}
		return 443, nil
	}
	port, err := strconv.Atoi(portText)
	if err != nil || port < 1 || port > 65535 {
		return 0, errModelPoolSSRFBlocked
	}
	return port, nil
}

func validateModelPoolEndpoint(ctx context.Context, parsed *url.URL, resolver modelPoolIPResolver, allowInsecureHTTP bool) error {
	if parsed == nil || (parsed.Scheme != "http" && parsed.Scheme != "https") || parsed.Host == "" || parsed.User != nil || parsed.RawQuery != "" || parsed.Fragment != "" {
		return errModelPoolSSRFBlocked
	}
	if parsed.Scheme == "http" && !allowInsecureHTTP {
		return errModelPoolInsecureHTTP
	}
	port, err := modelPoolEndpointPort(parsed)
	if err != nil {
		return err
	}
	if (parsed.Scheme == "http" && port != 80) || (parsed.Scheme == "https" && port != 443) {
		return errModelPoolSSRFBlocked
	}
	host := parsed.Hostname()
	if host == "" || strings.Contains(host, "%") {
		return errModelPoolSSRFBlocked
	}
	if resolver == nil {
		resolver = defaultModelPoolIPResolver
	}
	addresses, err := resolveModelPoolHost(ctx, host, resolver)
	if err != nil || len(addresses) == 0 {
		return fmt.Errorf("%w: resolve host", errModelPoolSSRFBlocked)
	}
	for _, address := range addresses {
		if isDisallowedModelPoolIP(address) {
			return fmt.Errorf("%w: resolved address", errModelPoolSSRFBlocked)
		}
	}
	return nil
}

func parseAndValidateModelPoolEndpoint(ctx context.Context, rawURL string, resolver modelPoolIPResolver, allowInsecureHTTP bool) (*url.URL, error) {
	parsed, err := url.Parse(rawURL)
	if err != nil {
		return nil, fmt.Errorf("%w: parse endpoint", errModelPoolSSRFBlocked)
	}
	if err := validateModelPoolEndpoint(ctx, parsed, resolver, allowInsecureHTTP); err != nil {
		return nil, err
	}
	return parsed, nil
}

func resolveModelPoolHost(ctx context.Context, host string, resolver modelPoolIPResolver) ([]netip.Addr, error) {
	if address, err := netip.ParseAddr(host); err == nil {
		return []netip.Addr{address.Unmap()}, nil
	}
	return resolver(ctx, host)
}

func prepareModelPoolHTTPClient(client *http.Client, resolver modelPoolIPResolver, allowInsecureHTTP bool) (*http.Client, error) {
	if client == nil {
		client = &http.Client{}
	}
	transport := client.Transport
	if transport == nil {
		transport = http.DefaultTransport
	}
	baseTransport, ok := transport.(*http.Transport)
	if !ok {
		return nil, errModelPoolTransport
	}
	safeTransport := baseTransport.Clone()
	// A proxy would make the actual destination the proxy rather than the
	// validated model host, so direct connections are required here.
	safeTransport.Proxy = nil
	safeTransport.MaxResponseHeaderBytes = modelPoolMaxResponseHeaderBytes
	// Force HTTPS through the guarded DialContext as well; otherwise a custom
	// DialTLS hook could bypass the resolved-address checks.
	safeTransport.DialTLSContext = nil
	safeTransport.DialTLS = nil
	dial := safeTransport.DialContext
	if dial == nil {
		dialer := &net.Dialer{Timeout: 10 * time.Second, KeepAlive: 30 * time.Second}
		dial = dialer.DialContext
	}
	safeTransport.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		host, port, err := net.SplitHostPort(address)
		if err != nil {
			return nil, fmt.Errorf("%w: invalid dial address", errModelPoolSSRFBlocked)
		}
		if port != "80" && port != "443" {
			return nil, fmt.Errorf("%w: invalid dial port", errModelPoolSSRFBlocked)
		}
		addresses, err := resolveModelPoolHost(ctx, host, resolver)
		if err != nil || len(addresses) == 0 {
			return nil, fmt.Errorf("%w: resolve dial host", errModelPoolSSRFBlocked)
		}
		var lastErr error
		for _, resolved := range addresses {
			if isDisallowedModelPoolIP(resolved) {
				return nil, fmt.Errorf("%w: disallowed dial address", errModelPoolSSRFBlocked)
			}
			connection, dialErr := dial(ctx, network, net.JoinHostPort(resolved.Unmap().String(), port))
			if dialErr == nil {
				return connection, nil
			}
			lastErr = dialErr
		}
		return nil, lastErr
	}

	safeClient := *client
	safeClient.Transport = safeTransport
	safeClient.CheckRedirect = func(req *http.Request, via []*http.Request) error {
		if len(via) >= modelPoolMaxRedirects {
			return errModelPoolRedirectLimit
		}
		if err := validateModelPoolEndpoint(req.Context(), req.URL, resolver, allowInsecureHTTP); err != nil {
			return err
		}
		return nil
	}
	return &safeClient, nil
}

type modelPoolModelsResponse struct {
	Data []struct {
		ID string `json:"id"`
	} `json:"data"`
}

func modelPoolResponseContainsModel(body []byte, model string) (bool, error) {
	var payload modelPoolModelsResponse
	if err := json.Unmarshal(body, &payload); err != nil {
		return false, err
	}
	for _, item := range payload.Data {
		if item.ID == model {
			return true, nil
		}
	}
	return false, nil
}

func isModelPoolTimeout(err error) bool {
	var timeoutError net.Error
	return errors.As(err, &timeoutError) && timeoutError.Timeout()
}
