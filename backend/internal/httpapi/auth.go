package httpapi

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"net/http"
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
}

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
	RefreshToken string `json:"refresh_token"`
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
	Actor            controlplane.Actor
	DeviceID         string
}

type authContextKey string

const sessionTokenContextKey authContextKey = "session_token"
const actorContextKey authContextKey = "actor"
const deviceContextKey authContextKey = "device"

type authenticator struct {
	mu           sync.Mutex
	controlPlane *service.ControlPlane
	config       AuthConfig
	sessions     map[string]sessionRecord
	refreshIndex map[string]string
	store        store.SessionStore
}

func newAuthenticator(controlPlane *service.ControlPlane, config AuthConfig, sessionStores ...store.SessionStore) *authenticator {
	var sessionStore store.SessionStore
	if len(sessionStores) > 0 {
		sessionStore = sessionStores[0]
	}
	return &authenticator{
		controlPlane: controlPlane,
		config:       config,
		sessions:     map[string]sessionRecord{},
		refreshIndex: map[string]string{},
		store:        sessionStore,
	}
}

func registerAuthRoutes(mux *http.ServeMux, auth *authenticator) {
	mux.Handle("POST /api/v1/auth/login", loginHandler(auth))
	mux.Handle("POST /api/v1/auth/refresh", refreshHandler(auth))
	mux.Handle("POST /api/v1/auth/logout", auth.requireBearer(logoutHandler(auth)))
}

func loginHandler(auth *authenticator) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.TrimSpace(auth.config.Username) == "" || auth.config.Password == "" {
			writeError(w, r, http.StatusServiceUnavailable, "AUTH_NOT_CONFIGURED", "开发阶段认证账号尚未配置")
			return
		}

		var request loginRequest
		if err := decodeJSONBody(r, &request); err != nil {
			writeError(w, r, http.StatusBadRequest, "INVALID_REQUEST", "登录请求格式无效")
			return
		}

		var actor controlplane.Actor
		var user controlplane.UserSummary
		if subtle.ConstantTimeCompare([]byte(request.Username), []byte(auth.config.Username)) == 1 &&
			subtle.ConstantTimeCompare([]byte(request.Password), []byte(auth.config.Password)) == 1 {
			auth.controlPlane.EnsureLocalAdmin(auth.config.Username)
			var err error
			user, err = auth.controlPlane.GetUser(r.Context(), "usr_local_admin")
			if err != nil {
				writeAppError(w, r, err)
				return
			}
			actor = controlplane.Actor{UserID: user.ID, Role: user.Role}
		} else {
			var err error
			actor, user, err = auth.controlPlane.AuthenticateUser(r.Context(), request.Username, request.Password)
			if err != nil {
				writeAppError(w, r, err)
				return
			}
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
			Actor:            controlplane.Actor{UserID: user.ID, Role: user.Role},
			DeviceID:         oldSession.DeviceID,
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
		auth.revokeAccessToken(r.Context(), token)
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

		user, err := a.controlPlane.GetUser(r.Context(), session.Actor.UserID)
		if err != nil {
			writeAppError(w, r, err)
			return
		}
		if user.Status != controlplane.UserStatusActive {
			writeError(w, r, http.StatusForbidden, "FORBIDDEN", "当前用户已不可用")
			return
		}

		actor := controlplane.Actor{UserID: user.ID, Role: user.Role}
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

func hashToken(token string) string {
	digest := sha256.Sum256([]byte(token))
	return base64.RawURLEncoding.EncodeToString(digest[:])
}

func (a *authenticator) revokeAccessToken(ctx context.Context, accessToken string) {
	if accessToken == "" {
		return
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
		_ = a.store.RevokeByAccessTokenHash(ctx, accessTokenHash)
	}
}

func (a *authenticator) bindDevice(r *http.Request, deviceID string) {
	token, _ := r.Context().Value(sessionTokenContextKey).(string)
	if token == "" || deviceID == "" {
		return
	}
	tokenHash := hashToken(token)
	var session sessionRecord
	var ok bool
	if a.store != nil {
		persisted, found, err := a.store.GetByAccessTokenHash(r.Context(), tokenHash)
		if err != nil || !found {
			return
		}
		session = sessionFromPersisted(persisted)
		ok = true
	} else {
		a.mu.Lock()
		session, ok = a.sessions[tokenHash]
		a.mu.Unlock()
	}
	if !ok {
		return
	}
	if session.DeviceID != "" && session.DeviceID != deviceID {
		return
	}
	if a.store != nil {
		_ = a.store.UpdateDeviceID(r.Context(), tokenHash, deviceID)
		return
	}
	session.DeviceID = deviceID
	a.mu.Lock()
	a.sessions[tokenHash] = session
	a.mu.Unlock()
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
		Actor:            controlplane.Actor{UserID: session.UserID},
		DeviceID:         session.DeviceID,
	}
}
