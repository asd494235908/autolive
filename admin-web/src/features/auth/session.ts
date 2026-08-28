import type { SessionTokens, UserSummary } from '../../types/api';

const SESSION_KEY = 'autolive.admin.session';

export type AdminSession = {
  tokens: SessionTokens;
  user: UserSummary;
};

export function readSession(): AdminSession | null {
  const raw = sessionStorage.getItem(SESSION_KEY);
  if (!raw) {
    return null;
  }

  try {
    const session = JSON.parse(raw) as AdminSession;
    if (
      !session.tokens?.access_token ||
      session.tokens.audience !== 'admin' ||
      !session.user?.id
    ) {
      sessionStorage.removeItem(SESSION_KEY);
      return null;
    }
    return session;
  } catch {
    sessionStorage.removeItem(SESSION_KEY);
    return null;
  }
}

export function saveSession(session: AdminSession) {
  sessionStorage.setItem(SESSION_KEY, JSON.stringify(session));
}

export function updateSessionTokens(tokens: SessionTokens) {
  const session = readSession();
  if (!session) {
    return;
  }
  saveSession({ ...session, tokens });
}

export function clearSession() {
  sessionStorage.removeItem(SESSION_KEY);
}

export function accessToken() {
  return readSession()?.tokens.access_token;
}
