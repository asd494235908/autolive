package httpapi

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"errors"
	"fmt"
	"net/http"
	"regexp"
	"strings"
	"sync"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/service"
	"autoLive/backend/internal/store"
)

type loginRequest struct {
	Username string `json:"username"`
	Password string `json:"password"`
	Product  string `json:"product"`
}

var deviceIDPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_-]{7,63}$`)

type loginResponse struct {
	RequestID string        `json:"request_id"`
	Tokens    sessionTokens `json:"tokens"`
	User      userSummary   `json:"user"`
}

type refreshTokenRequest struct {
	RefreshToken string `json:"refresh_token"`
}

type refreshTokenResponse struct {
	RequestID string        `json:"request_id"`
	Tokens    sessionTokens `json:"tokens"`
}

type logoutRequest struct {
	RefreshToken string `json:"refresh_token,omitempty"`
}

type logoutResponse struct {
	RequestID string `json:"request_id"`
	Success   bool   `json:"success"`
}

type sessionTokens struct {
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
	ExpiresAt    string `json:"expires_at"`
}

type userSummary struct {
	ID        string `json:"id"`
	Username  string `json:"username"`
	Role      string `json:"role"`
	Status    string `json:"status"`
	CreatedAt string `json:"created_at"`
}

type sessionRecord struct {
	AccessTokenHash  string
	RefreshTokenHash string
	ExpiresAt        time.Time
	RefreshExpiresAt time.Time
	Product          controlplane.ProductCode
	Actor            controlplane.Actor
	DeviceID         string
}

type authContextKey string

const sessionTokenContextKey authContextKey = "session_token"
const actorContextKey authContextKey = "actor"
const deviceContextKey authContextKey = "device"

const legacyClientCompatibilityHeader = "X-Client-Compatibility"

type authenticator struct {
	mu           sync.Mutex
	controlPlane *service.ControlPlane
	initErr      error
	sessions     map[string]sessionRecord
	refreshIndex map[string]string
	store        store.SessionStore
	productRepo  store.ProductRepository
}

func newAuthenticator(controlPlane *service.ControlPlane, config AuthConfig, sessionStores ...store.SessionStore) *authenticator {
	return newAuthenticatorWithProductRepository(controlPlane, config, nil, sessionStores...)
}

func newAuthenticatorWithProductRepository(controlPlane *service.ControlPlane, config AuthConfig, productRepo store.ProductRepository, sessionStores ...store.SessionStore) *authenticator {
	var sessionStore store.SessionStore
	if len(sessionStores) > 0 {
		sessionStore = sessionStores[0]
	}
	var initializationErr error
	if !(config.UsePersistedAdmin && strings.TrimSpace(config.Username) == "" && config.Password == "") {
		initializationErr = controlPlane.EnsureConfiguredAdmin(context.Background(), config.Username, config.Password)
	}
	config.Password = ""
	return &authenticator{
		controlPlane: controlPlane,
		initErr:      initializationErr,
		sessions:     map[string]sessionRecord{},
		refreshIndex: map[string]string{},
		store:        sessionStore,
		productRepo:  productRepo,
	}
}

func registerAuthRoutes(mux *http.ServeMux, auth *authenticator) {
	mux.Handle("POST /api/v1/auth/login", loginHandler(auth))
	mux.Handle("POST /api/v1/auth/refresh", refreshHandler(auth))
	mux.Handle("POST /api/v1/auth/logout", auth.requireBearer(logoutHandler(auth)))
}

func loginHandler(auth *authenticator) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if auth.initErr != nil {
			writeError(w, r, http.StatusServiceUnavailable, "AUTH_INITIALIZATION_UNAVAILABLE", "管理员认证初始化暂不可用")
			return
		}

		var request loginRequest
		if err := decodeJSONBody(r, &request); err != nil {
			writeError(w, r, http.StatusBadRequest, "INVALID_REQUEST", "登录请求格式无效")
			return
		}
		product, err := loginProduct(r, request.Product)
		if err != nil {
			writeError(w, r, http.StatusBadRequest, "INVALID_REQUEST", "登录请求格式无效")
			return
		}

		actor, user, err := auth.controlPlane.AuthenticateUser(r.Context(), request.Username, request.Password)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		if auth.productRepo != nil {
			membership, err := auth.productRepo.GetUserProductMembership(r.Context(), user.ID, product)
			if err != nil {
				if errors.Is(err, controlplane.ErrForbidden) {
					writeAppError(w, r, controlplane.ErrForbidden)
				} else {
					writeError(w, r, http.StatusServiceUnavailable, "PRODUCT_MEMBERSHIP_UNAVAILABLE", "产品授权暂时无法读取")
				}
				return
			}
			if membership.Status != "active" {
				writeAppError(w, r, controlplane.ErrForbidden)
				return
			}
		}
		actor.Product = product
		if principal, ok := r.Context().Value(auditPrincipalContextKey{}).(*auditPrincipal); ok {
			principal.actor = actor
			principal.set = true
		}

		accessToken, err := newToken()
		if err != nil {
			writeError(w, r, http.StatusInternalServerError, "TOKEN_GENERATION_FAILED", "无法生成登录凭证")
			return
		}
		refreshToken, err := newToken()
		if err != nil {
			writeError(w, r, http.StatusInternalServerError, "TOKEN_GENERATION_FAILED", "无法生成登录凭证")
			return
		}

		now := time.Now().UTC()
		session := sessionRecord{
			AccessTokenHash:  hashToken(accessToken),
			RefreshTokenHash: hashToken(refreshToken),
			ExpiresAt:        now.Add(time.Hour),
			RefreshExpiresAt: now.Add(30 * 24 * time.Hour),
			Product:          product,
			Actor:            actor,
		}
		if auth.store != nil {
			if err := auth.store.Create(r.Context(), persistedSession("session_"+session.AccessTokenHash, actor.UserID, session)); err != nil {
				writeError(w, r, http.StatusServiceUnavailable, "AUTH_SESSION_STORE_UNAVAILABLE", "登录会话暂时无法保存")
				return
			}
		}

		if auth.store == nil {
			auth.mu.Lock()
			auth.sessions[session.AccessTokenHash] = session
			auth.refreshIndex[session.RefreshTokenHash] = session.AccessTokenHash
			auth.mu.Unlock()
		}

		writeJSON(w, http.StatusOK, loginResponse{
			RequestID: RequestIDFromContext(r.Context()),
			Tokens: sessionTokens{
				AccessToken:  accessToken,
				RefreshToken: refreshToken,
				ExpiresAt:    session.ExpiresAt.Format(time.RFC3339),
			},
			User: userSummary{
				ID:        user.ID,
				Username:  user.Username,
				Role:      user.Role,
				Status:    user.Status,
				CreatedAt: user.CreatedAt,
			},
		})
	})
}

func refreshHandler(auth *authenticator) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var request refreshTokenRequest
		if err := decodeJSONBody(r, &request); err != nil || strings.TrimSpace(request.RefreshToken) == "" {
			writeError(w, r, http.StatusBadRequest, "INVALID_REQUEST", "刷新请求格式无效")
			return
		}

		newAccessToken, err := newToken()
		if err != nil {
			writeError(w, r, http.StatusInternalServerError, "TOKEN_GENERATION_FAILED", "无法生成登录凭证")
			return
		}
		newRefreshToken, err := newToken()
		if err != nil {
			writeError(w, r, http.StatusInternalServerError, "TOKEN_GENERATION_FAILED", "无法生成登录凭证")
			return
		}

		now := time.Now().UTC()
		refreshHash := hashToken(request.RefreshToken)
		var oldSession sessionRecord
		var oldAccessTokenHash string
		if auth.store != nil {
			persisted, found, err := auth.store.GetByRefreshTokenHash(r.Context(), refreshHash)
			if err != nil {
				writeError(w, r, http.StatusServiceUnavailable, "AUTH_SESSION_STORE_UNAVAILABLE", "刷新会话暂时无法读取")
				return
			}
			if !found || now.After(persisted.RefreshExpiresAt) {
				writeError(w, r, http.StatusUnauthorized, "UNAUTHENTICATED", "刷新凭证已失效或不存在")
				return
			}
			oldSession = sessionFromPersisted(persisted)
			oldAccessTokenHash = oldSession.AccessTokenHash
		} else {
			auth.mu.Lock()
			var ok, sessionExists bool
			oldAccessTokenHash, ok = auth.refreshIndex[refreshHash]
			oldSession, sessionExists = auth.sessions[oldAccessTokenHash]
			auth.mu.Unlock()
			if !ok || !sessionExists || now.After(oldSession.RefreshExpiresAt) {
				writeError(w, r, http.StatusUnauthorized, "UNAUTHENTICATED", "刷新凭证已失效或不存在")
				return
			}
		}

		user, err := auth.controlPlane.GetUser(r.Context(), oldSession.Actor.UserID)
		if err != nil || user.Status != controlplane.UserStatusActive {
			writeError(w, r, http.StatusUnauthorized, "UNAUTHENTICATED", "用户会话已失效")
			return
		}

		newSession := sessionRecord{
			AccessTokenHash:  hashToken(newAccessToken),
			RefreshTokenHash: hashToken(newRefreshToken),
			ExpiresAt:        now.Add(time.Hour),
			RefreshExpiresAt: oldSession.RefreshExpiresAt,
			Product:          oldSession.Product,
			Actor:            controlplane.Actor{UserID: user.ID, Role: user.Role, Product: oldSession.Product},
			DeviceID:         oldSession.DeviceID,
		}
		if !newSession.Product.Valid() {
			writeError(w, r, http.StatusUnauthorized, "UNAUTHENTICATED", "会话产品无效")
			return
		}
		if auth.store != nil {
			rotated, found, err := auth.store.Rotate(r.Context(), refreshHash, persistedSession("session_"+newSession.AccessTokenHash, user.ID, newSession))
			if err != nil {
				writeError(w, r, http.StatusServiceUnavailable, "AUTH_SESSION_STORE_UNAVAILABLE", "刷新会话暂时无法保存")
				return
			}
			if !found {
				writeError(w, r, http.StatusUnauthorized, "UNAUTHENTICATED", "刷新凭证已失效或不存在")
				return
			}
			oldAccessTokenHash = rotated.AccessTokenHash
		} else {
			auth.mu.Lock()
			delete(auth.sessions, oldAccessTokenHash)
			delete(auth.refreshIndex, oldSession.RefreshTokenHash)
			auth.sessions[newSession.AccessTokenHash] = newSession
			auth.refreshIndex[newSession.RefreshTokenHash] = newSession.AccessTokenHash
			auth.mu.Unlock()
		}

		writeJSON(w, http.StatusOK, refreshTokenResponse{
			RequestID: RequestIDFromContext(r.Context()),
			Tokens: sessionTokens{
				AccessToken:  newAccessToken,
				RefreshToken: newRefreshToken,
				ExpiresAt:    newSession.ExpiresAt.Format(time.RFC3339),
			},
		})
	})
}

func logoutHandler(auth *authenticator) func(http.ResponseWriter, *http.Request, controlplane.Actor) {
	return func(w http.ResponseWriter, r *http.Request, _ controlplane.Actor) {
		var request logoutRequest
		if err := decodeOptionalJSONBody(r, &request); err != nil {
			writeError(w, r, http.StatusBadRequest, "INVALID_REQUEST", "退出请求格式无效")
			return
		}
		token, _ := r.Context().Value(sessionTokenContextKey).(string)
		if err := auth.revokeAccessToken(r.Context(), token); err != nil {
			writeError(w, r, http.StatusServiceUnavailable, "AUTH_SESSION_STORE_UNAVAILABLE", "登录会话暂时无法注销")
			return
		}
		writeJSON(w, http.StatusOK, logoutResponse{
			RequestID: RequestIDFromContext(r.Context()),
			Success:   true,
		})
	}
}

func (a *authenticator) requireBearer(next func(http.ResponseWriter, *http.Request, controlplane.Actor)) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		parts := strings.Fields(r.Header.Get("Authorization"))
		if len(parts) != 2 || !strings.EqualFold(parts[0], "Bearer") || strings.TrimSpace(parts[1]) == "" {
			writeError(w, r, http.StatusUnauthorized, "UNAUTHENTICATED", "未提供有效 Bearer Token")
			return
		}
		token := parts[1]

		tokenHash := hashToken(token)
		var session sessionRecord
		var ok bool
		if a.store != nil {
			persisted, found, err := a.store.GetByAccessTokenHash(r.Context(), tokenHash)
			if err != nil {
				writeError(w, r, http.StatusServiceUnavailable, "AUTH_SESSION_STORE_UNAVAILABLE", "登录会话暂时无法读取")
				return
			}
			if found {
				session = sessionFromPersisted(persisted)
				ok = true
			}
		} else {
			a.mu.Lock()
			session, ok = a.sessions[tokenHash]
			a.mu.Unlock()
		}
		if !ok || time.Now().UTC().After(session.ExpiresAt) {
			writeError(w, r, http.StatusUnauthorized, "UNAUTHENTICATED", "会话已失效或不存在")
			return
		}
		if !session.Product.Valid() {
			writeError(w, r, http.StatusUnauthorized, "UNAUTHENTICATED", "会话产品无效")
			return
		}

		user, err := a.controlPlane.GetUser(r.Context(), session.Actor.UserID)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		if user.Status != controlplane.UserStatusActive {
			writeError(w, r, http.StatusForbidden, "FORBIDDEN", "当前用户已不可用")
			return
		}

		actor := controlplane.Actor{UserID: user.ID, Role: user.Role, Product: session.Product}
		ctx := context.WithValue(r.Context(), sessionTokenContextKey, token)
		ctx = context.WithValue(ctx, actorContextKey, actor)
		ctx = context.WithValue(ctx, deviceContextKey, session.DeviceID)
		if principal, ok := ctx.Value(auditPrincipalContextKey{}).(*auditPrincipal); ok {
			principal.actor = actor
			principal.deviceID = session.DeviceID
			principal.set = true
		}
		next(w, r.WithContext(ctx), actor)
	})
}

func (a *authenticator) requireAdmin(next func(http.ResponseWriter, *http.Request, controlplane.Actor)) http.Handler {
	return a.requireBearer(func(w http.ResponseWriter, r *http.Request, actor controlplane.Actor) {
		if actor.Role != controlplane.RoleAdmin {
			writeError(w, r, http.StatusForbidden, "FORBIDDEN", "当前身份不是管理员")
			return
		}
		next(w, r, actor)
	})
}

func newToken() (string, error) {
	var token [24]byte
	if _, err := rand.Read(token[:]); err != nil {
		return "", err
	}

	return base64.RawURLEncoding.EncodeToString(token[:]), nil
}

func loginProduct(r *http.Request, raw string) (controlplane.ProductCode, error) {
	if strings.TrimSpace(raw) == "" && strings.EqualFold(strings.TrimSpace(r.Header.Get(legacyClientCompatibilityHeader)), "legacy") {
		return controlplane.ProductAutoLive, nil
	}
	return controlplane.ParseProductCode(raw)
}

func hashToken(token string) string {
	digest := sha256.Sum256([]byte(token))
	return base64.RawURLEncoding.EncodeToString(digest[:])
}

func (a *authenticator) revokeAccessToken(ctx context.Context, accessToken string) error {
	if accessToken == "" {
		return errors.New("access token is required")
	}
	accessTokenHash := hashToken(accessToken)
	a.mu.Lock()
	session, ok := a.sessions[accessTokenHash]
	if ok {
		delete(a.sessions, accessTokenHash)
		delete(a.refreshIndex, session.RefreshTokenHash)
	}
	a.mu.Unlock()
	if a.store != nil {
		if err := a.store.RevokeByAccessTokenHash(ctx, accessTokenHash); err != nil {
			return fmt.Errorf("revoke auth session: %w", err)
		}
	}
	return nil
}

func (a *authenticator) revokeUserSessions(ctx context.Context, userID string) error {
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return errors.New("user id is required")
	}
	if a.store != nil {
		if err := a.store.RevokeByUserID(ctx, userID); err != nil {
			return fmt.Errorf("revoke user sessions: %w", err)
		}
		return nil
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	for accessHash, session := range a.sessions {
		if session.Actor.UserID == userID {
			delete(a.sessions, accessHash)
			delete(a.refreshIndex, session.RefreshTokenHash)
		}
	}
	return nil
}

func (a *authenticator) revokeDeviceSessions(ctx context.Context, deviceID string) error {
	return a.revokeDeviceSessionsForProduct(ctx, deviceID, controlplane.ProductAutoLive)
}

func (a *authenticator) revokeDeviceSessionsForProduct(ctx context.Context, deviceID string, product controlplane.ProductCode) error {
	deviceID = strings.TrimSpace(deviceID)
	if deviceID == "" || !product.Valid() {
		return errors.New("device id is required")
	}
	if a.store != nil {
		if err := a.store.RevokeByDeviceIDForProduct(ctx, deviceID, product); err != nil {
			return fmt.Errorf("revoke device sessions: %w", err)
		}
		return nil
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	for accessHash, session := range a.sessions {
		if session.DeviceID == deviceID && session.Actor.Product == product {
			delete(a.sessions, accessHash)
			delete(a.refreshIndex, session.RefreshTokenHash)
		}
	}
	return nil
}

func (a *authenticator) bindDevice(r *http.Request, deviceID string) error {
	_, err := a.bindDeviceTracked(r, deviceID)
	return err
}

// accessTokenHash returns the already-authenticated token lookup key for the
// optional SQL transaction coordinator. The token itself never crosses the
// service boundary or gets persisted.
func (a *authenticator) accessTokenHash(r *http.Request) string {
	token, _ := r.Context().Value(sessionTokenContextKey).(string)
	if token == "" {
		return ""
	}
	return hashToken(token)
}

// bindDeviceTracked returns true only when this request changed an unbound
// session. Callers can then clear that exact binding if a later operation in
// the same HTTP flow fails, without disturbing an already-bound session.
func (a *authenticator) bindDeviceTracked(r *http.Request, deviceID string) (bool, error) {
	token, _ := r.Context().Value(sessionTokenContextKey).(string)
	if token == "" {
		return false, errors.New("access token and device id are required")
	}
	if !deviceIDPattern.MatchString(deviceID) {
		return false, controlplane.ErrInvalidRequest
	}
	tokenHash := hashToken(token)
	var session sessionRecord
	var ok bool
	if a.store != nil {
		persisted, found, err := a.store.GetByAccessTokenHash(r.Context(), tokenHash)
		if err != nil {
			return false, fmt.Errorf("read auth session for device binding: %w", err)
		}
		if !found {
			return false, errors.New("auth session not found for device binding")
		}
		session = sessionFromPersisted(persisted)
		ok = true
	} else {
		a.mu.Lock()
		session, ok = a.sessions[tokenHash]
		a.mu.Unlock()
	}
	if !ok {
		return false, errors.New("auth session not found for device binding")
	}
	if session.DeviceID != "" && session.DeviceID != deviceID {
		return false, controlplane.ErrDeviceBindingConflict
	}
	if session.DeviceID == deviceID {
		setAuditDeviceID(r, deviceID)
		return false, nil
	}
	if a.store != nil {
		if err := a.store.UpdateDeviceID(r.Context(), tokenHash, deviceID); err != nil {
			return false, fmt.Errorf("bind device to auth session: %w", err)
		}
		setAuditDeviceID(r, deviceID)
		return true, nil
	}
	session.DeviceID = deviceID
	a.mu.Lock()
	a.sessions[tokenHash] = session
	a.mu.Unlock()
	setAuditDeviceID(r, deviceID)
	return true, nil
}

func setAuditDeviceID(r *http.Request, deviceID string) {
	if principal, ok := r.Context().Value(auditPrincipalContextKey{}).(*auditPrincipal); ok {
		principal.deviceID = deviceID
	}
}

func (a *authenticator) clearDeviceBinding(r *http.Request, deviceID string) error {
	compensationCtx, cancel := context.WithTimeout(context.WithoutCancel(r.Context()), 2*time.Second)
	defer cancel()
	token, _ := r.Context().Value(sessionTokenContextKey).(string)
	if token == "" || deviceID == "" {
		return errors.New("access token and device id are required")
	}
	tokenHash := hashToken(token)
	if a.store != nil {
		if err := a.store.ClearDeviceID(compensationCtx, tokenHash, deviceID); err != nil {
			return fmt.Errorf("clear auth session device binding: %w", err)
		}
		return nil
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	session, ok := a.sessions[tokenHash]
	if !ok || session.DeviceID != deviceID {
		return errors.New("auth session device binding not found")
	}
	session.DeviceID = ""
	a.sessions[tokenHash] = session
	return nil
}

func (a *authenticator) deviceID(r *http.Request) string {
	token, _ := r.Context().Value(sessionTokenContextKey).(string)
	if token == "" {
		return ""
	}
	tokenHash := hashToken(token)
	if a.store != nil {
		persisted, found, err := a.store.GetByAccessTokenHash(r.Context(), tokenHash)
		if err != nil || !found {
			return ""
		}
		return persisted.DeviceID
	}
	a.mu.Lock()
	session, ok := a.sessions[tokenHash]
	a.mu.Unlock()
	if !ok {
		return ""
	}
	return session.DeviceID
}

func persistedSession(id, userID string, session sessionRecord) store.AuthSession {
	return store.AuthSession{
		ID:               id,
		UserID:           userID,
		Product:          session.Product,
		DeviceID:         session.DeviceID,
		AccessTokenHash:  session.AccessTokenHash,
		RefreshTokenHash: session.RefreshTokenHash,
		AccessExpiresAt:  session.ExpiresAt,
		RefreshExpiresAt: session.RefreshExpiresAt,
		CreatedAt:        time.Now().UTC(),
	}
}

func sessionFromPersisted(session store.AuthSession) sessionRecord {
	return sessionRecord{
		AccessTokenHash:  session.AccessTokenHash,
		RefreshTokenHash: session.RefreshTokenHash,
		ExpiresAt:        session.AccessExpiresAt,
		RefreshExpiresAt: session.RefreshExpiresAt,
		Product:          session.Product,
		Actor:            controlplane.Actor{UserID: session.UserID, Product: session.Product},
		DeviceID:         session.DeviceID,
	}
}
