using GpAutoLive.Contracts;
using GpAutoLive.Core;

namespace GpAutoLive.Core.Tests;

[TestClass]
public sealed class AuthSessionTests
{
    private static readonly DateTimeOffset Now = new(2026, 9, 2, 8, 0, 0, TimeSpan.Zero);

    [TestMethod]
    public void Unauthenticated_session_cannot_activate_or_enter_workbench()
    {
        var machine = new AuthSessionMachine();

        var transition = machine.BeginActivation("activate-unauth", Now);

        Assert.IsFalse(transition.IsSuccess);
        Assert.AreEqual(AuthErrorCodes.Unauthenticated, transition.Error!.Code);
        Assert.IsFalse(machine.Snapshot.CanEnterWorkbench);
    }

    [TestMethod]
    public void Login_then_active_activation_is_the_only_ready_gate()
    {
        var machine = LoggedInMachine();

        var begin = machine.BeginActivation("activate-1", Now);
        var complete = machine.CompleteActivation("activate-1", ActiveDevice(), Now);

        Assert.IsTrue(begin.IsSuccess);
        Assert.IsTrue(complete.IsSuccess);
        Assert.AreEqual(AuthSessionState.Activated, machine.Snapshot.State);
        Assert.IsTrue(machine.Snapshot.CanEnterWorkbench);
    }

    [TestMethod]
    public void Cold_restore_binds_user_only_after_active_device_response()
    {
        var machine = new AuthSessionMachine();
        var begin = machine.BeginRestore("restore-1", "device01", Now);
        var restored = machine.CompleteRestore(
            "restore-1",
            new AuthTokenSet("access", "refresh", Now.AddMinutes(15), Now.AddDays(30), ControlPlaneContractValues.DesktopAudience),
            Now);
        var activationBegin = machine.BeginActivation("activate-after-restore", Now);
        var activated = machine.CompleteActivation("activate-after-restore", ActiveDevice(), Now);

        Assert.IsTrue(begin.IsSuccess);
        Assert.IsTrue(restored.IsSuccess);
        Assert.IsTrue(activationBegin.IsSuccess);
        Assert.AreEqual(AuthTransitionKind.Completed, activated.Kind);
        Assert.AreEqual(AuthSessionState.Activated, machine.Snapshot.State);
        Assert.AreEqual("user-1", machine.Snapshot.UserId);
    }

    [TestMethod]
    public void Cancelling_restore_clears_pending_without_deleting_credential()
    {
        var machine = new AuthSessionMachine();
        _ = machine.BeginRestore("restore-cancel", "device01", Now);

        var cancelled = machine.CancelPendingOperation();

        Assert.IsNotNull(cancelled);
        Assert.AreEqual(AuthOperationKind.Restore, cancelled!.Operation);
        Assert.AreEqual("CONTROL_PLANE_CANCELLED", cancelled.Error!.Code);
        Assert.AreEqual(AuthSessionState.Unauthenticated, machine.Snapshot.State);
        Assert.IsTrue(machine.BeginLogin(
            "login-after-cancel",
            "device01",
            new DesktopLoginRequestDto("alice", "password-123", ControlPlaneContractValues.Product),
            Now).IsSuccess);
    }

    [TestMethod]
    public void Disabled_device_keeps_login_but_never_becomes_ready()
    {
        var machine = LoggedInMachine();
        _ = machine.BeginActivation("activate-disabled", Now);

        var transition = machine.CompleteActivation(
            "activate-disabled",
            ActiveDevice() with { Status = "disabled" },
            Now);

        Assert.IsFalse(transition.IsSuccess);
        Assert.AreEqual(AuthErrorCodes.DeviceDisabled, transition.Error!.Code);
        Assert.AreEqual(AuthSessionState.Authenticated, machine.Snapshot.State);
        Assert.IsTrue(machine.Snapshot.IsAuthenticated);
        Assert.IsFalse(machine.Snapshot.CanEnterWorkbench);
    }

    [TestMethod]
    public void Activation_without_expiry_is_rejected_fail_closed()
    {
        var machine = LoggedInMachine();
        _ = machine.BeginActivation("activate-missing-expiry", Now);

        var transition = machine.CompleteActivation(
            "activate-missing-expiry",
            ActiveDevice() with { ActivationExpiresAt = null },
            Now);

        Assert.IsFalse(transition.IsSuccess);
        Assert.AreEqual(AuthErrorCodes.ResponseInvalid, transition.Error!.Code);
        Assert.AreEqual(AuthSessionState.Authenticated, machine.Snapshot.State);
        Assert.IsTrue(machine.Snapshot.IsAuthenticated);
        Assert.IsFalse(machine.Snapshot.CanEnterWorkbench);
    }

    [TestMethod]
    public void Device_capacity_error_keeps_login_session_for_activation_retry()
    {
        var machine = LoggedInMachine();
        _ = machine.BeginActivation("activate-capacity", Now);

        var transition = machine.FailActivation(
            "activate-capacity",
            new ControlPlaneErrorDto(AuthErrorCodes.DeviceLimitExceeded, "设备数量已达到授权上限", 409),
            Now);

        Assert.IsFalse(transition.IsSuccess);
        Assert.AreEqual(AuthSessionState.Authenticated, transition.Snapshot.State);
        Assert.IsTrue(transition.Snapshot.IsAuthenticated);
        Assert.IsNull(transition.CredentialAction);
        Assert.IsFalse(transition.Snapshot.CanEnterWorkbench);
    }

    [TestMethod]
    public void Activated_session_expires_at_runtime_without_waiting_for_server()
    {
        var machine = ActivatedMachine();

        var before = machine.ExpireActivationIfNeeded("expiry-before", Now.AddDays(29));
        var expired = machine.ExpireActivationIfNeeded("expiry-now", Now.AddDays(30));

        Assert.IsNull(before);
        Assert.IsNotNull(expired);
        Assert.AreEqual(AuthErrorCodes.AccountActivationExpired, expired!.Error!.Code);
        Assert.IsNull(expired.CredentialAction);
        Assert.AreEqual(AuthSessionState.Authenticated, machine.Snapshot.State);
        Assert.IsTrue(machine.Snapshot.IsAuthenticated);
        Assert.IsFalse(machine.Snapshot.CanEnterWorkbench);
    }

    [TestMethod]
    public void Unauthenticated_activation_error_is_the_only_activation_failure_that_clears_login()
    {
        var machine = LoggedInMachine();
        _ = machine.BeginActivation("activate-unauthenticated", Now);

        var transition = machine.FailActivation(
            "activate-unauthenticated",
            new ControlPlaneErrorDto(AuthErrorCodes.Unauthenticated, "会话已失效", 401),
            Now);

        Assert.AreEqual(AuthSessionState.Unauthenticated, transition.Snapshot.State);
        Assert.AreEqual(CredentialActionKind.DeleteRefreshToken, transition.CredentialAction!.Kind);
        Assert.IsNull(transition.Snapshot.AccessToken);
        Assert.IsNull(transition.Snapshot.UserId);
        Assert.IsNull(transition.Snapshot.ActivationExpiresAt);
    }

    [TestMethod]
    public void Offline_heartbeat_uses_bounded_backoff_and_same_key_can_retry()
    {
        var machine = ActivatedMachine();
        var request = Heartbeat();
        _ = machine.BeginHeartbeat("heartbeat-1", request, Now);

        var failed = machine.FailHeartbeat("heartbeat-1", new ControlPlaneErrorDto("NETWORK_UNAVAILABLE", "网络不可用", 503), Now);
        var tooEarly = machine.BeginHeartbeat("heartbeat-1", request, Now.AddMilliseconds(500));
        var retry = machine.BeginHeartbeat("heartbeat-1", request, Now.AddSeconds(1));
        var recovered = machine.CompleteHeartbeat("heartbeat-1", "active", Now.AddSeconds(1));

        Assert.AreEqual(AuthTransitionKind.RetryScheduled, failed.Kind);
        Assert.AreEqual(AuthSessionState.Offline, failed.Snapshot.State);
        Assert.AreEqual(1, failed.Snapshot.RetryAttempt);
        Assert.AreEqual(AuthTransitionKind.RetryNotDue, tooEarly.Kind);
        Assert.AreEqual(AuthErrorCodes.RetryNotDue, tooEarly.Error!.Code);
        Assert.AreEqual(AuthTransitionKind.Accepted, retry.Kind);
        Assert.AreEqual(AuthTransitionKind.Completed, recovered.Kind);
        Assert.AreEqual(AuthSessionState.Activated, machine.Snapshot.State);
    }

    [TestMethod]
    [DataRow(AuthErrorCodes.InvalidRequest, 400)]
    [DataRow("HEARTBEAT_CONFLICT", 409)]
    public void Heartbeat_telemetry_error_keeps_activated_session(string code, int status)
    {
        var machine = ActivatedMachine();
        _ = machine.BeginHeartbeat("heartbeat-telemetry", Heartbeat(), Now);

        var failed = machine.FailHeartbeat(
            "heartbeat-telemetry",
            new ControlPlaneErrorDto(code, "心跳遥测被拒绝", status),
            Now);

        Assert.AreEqual(AuthSessionState.Activated, failed.Snapshot.State);
        Assert.IsTrue(failed.Snapshot.CanEnterWorkbench);
        Assert.IsNull(failed.CredentialAction);
    }

    [TestMethod]
    public void Heartbeat_telemetry_error_keeps_existing_offline_session()
    {
        var machine = ActivatedMachine();
        _ = machine.BeginHeartbeat("heartbeat-offline", Heartbeat(), Now);
        _ = machine.FailHeartbeat(
            "heartbeat-offline",
            new ControlPlaneErrorDto("NETWORK_UNAVAILABLE", "网络不可用", 503),
            Now);
        var retryAt = Now.Add(AuthSessionMachine.RetryDelayForAttempt(1));
        _ = machine.BeginHeartbeat("heartbeat-offline-telemetry", Heartbeat(), retryAt);

        var failed = machine.FailHeartbeat(
            "heartbeat-offline-telemetry",
            new ControlPlaneErrorDto(AuthErrorCodes.InvalidRequest, "心跳遥测无效", 400),
            retryAt);

        Assert.AreEqual(AuthSessionState.Offline, failed.Snapshot.State);
        Assert.IsTrue(failed.Snapshot.IsAuthenticated);
        Assert.IsNull(failed.CredentialAction);
    }

    [TestMethod]
    [DataRow(AuthErrorCodes.Unauthenticated, 403)]
    [DataRow("ACCESS_TOKEN_EXPIRED", 401)]
    public void Heartbeat_unauthenticated_keeps_refreshable_offline_session(string code, int status)
    {
        var machine = ActivatedMachine();
        _ = machine.BeginHeartbeat("heartbeat-unauthenticated", Heartbeat(), Now);

        var failed = machine.FailHeartbeat(
            "heartbeat-unauthenticated",
            new ControlPlaneErrorDto(code, "访问凭据已失效", status),
            Now);

        Assert.AreEqual(AuthSessionState.Offline, failed.Snapshot.State);
        Assert.IsTrue(failed.Snapshot.IsAuthenticated);
        Assert.IsTrue(failed.Snapshot.CanContinueLocalPlayback);
        Assert.IsNull(failed.CredentialAction);
    }

    [TestMethod]
    [DataRow(AuthErrorCodes.AccountActivationRequired, 403)]
    [DataRow(AuthErrorCodes.AccountActivationExpired, 403)]
    [DataRow("FORBIDDEN", 403)]
    [DataRow(AuthErrorCodes.DeviceNotFound, 404)]
    [DataRow(AuthErrorCodes.DeviceBindingRequired, 403)]
    [DataRow(AuthErrorCodes.DeviceBindingConflict, 409)]
    [DataRow(AuthErrorCodes.DeviceLimitExceeded, 409)]
    public void Heartbeat_authorization_error_closes_workbench_but_keeps_session(string code, int status)
    {
        var machine = ActivatedMachine();
        _ = machine.BeginHeartbeat("heartbeat-authorization", Heartbeat(), Now);

        var failed = machine.FailHeartbeat(
            "heartbeat-authorization",
            new ControlPlaneErrorDto(code, "设备授权无效", status),
            Now);

        Assert.AreEqual(AuthSessionState.Authenticated, failed.Snapshot.State);
        Assert.IsTrue(failed.Snapshot.IsAuthenticated);
        Assert.IsFalse(failed.Snapshot.CanEnterWorkbench);
        Assert.IsNull(failed.Snapshot.ActivationExpiresAt);
        Assert.IsNull(failed.CredentialAction);
    }

    [TestMethod]
    [DataRow(AuthErrorCodes.DeviceDisabled)]
    [DataRow(AuthErrorCodes.DeviceRevoked)]
    public void Heartbeat_disabled_device_stays_fail_closed(string code)
    {
        var machine = ActivatedMachine();
        _ = machine.BeginHeartbeat("heartbeat-disabled", Heartbeat(), Now);

        var failed = machine.FailHeartbeat(
            "heartbeat-disabled",
            new ControlPlaneErrorDto(code, "设备不可用", 403),
            Now);

        Assert.AreEqual(AuthSessionState.Disabled, failed.Snapshot.State);
        Assert.IsFalse(failed.Snapshot.IsAuthenticated);
        Assert.IsFalse(failed.Snapshot.CanEnterWorkbench);
    }

    [TestMethod]
    public void Completed_operation_is_idempotent_and_conflicting_key_is_rejected()
    {
        var machine = ActivatedMachine();
        var request = Heartbeat();
        _ = machine.BeginHeartbeat("heartbeat-idem", request, Now);
        var first = machine.CompleteHeartbeat("heartbeat-idem", "active", Now);
        var replay = machine.CompleteHeartbeat("heartbeat-idem", "active", Now.AddSeconds(1));
        var conflict = machine.BeginLogin(
            "heartbeat-idem",
            "device01",
            new DesktopLoginRequestDto("alice", "password-123", ControlPlaneContractValues.Product),
            Now);

        Assert.AreEqual(AuthTransitionKind.Completed, first.Kind);
        Assert.AreEqual(AuthTransitionKind.Duplicate, replay.Kind);
        Assert.AreEqual(AuthTransitionKind.Rejected, conflict.Kind);
        Assert.AreEqual(AuthErrorCodes.IdempotencyConflict, conflict.Error!.Code);
    }

    [TestMethod]
    public void Expired_refresh_token_is_never_emitted_as_store_action()
    {
        var machine = new AuthSessionMachine();
        _ = machine.BeginLogin(
            "login-expired-refresh",
            "device01",
            new DesktopLoginRequestDto("alice", "password-123", ControlPlaneContractValues.Product),
            Now);

        var transition = machine.CompleteLogin(
            "login-expired-refresh",
            new AuthTokenSet("access", "refresh", Now.AddMinutes(15), Now.AddSeconds(-1), ControlPlaneContractValues.DesktopAudience),
            User(),
            Now);

        Assert.IsFalse(transition.IsSuccess);
        Assert.AreEqual(AuthErrorCodes.ResponseInvalid, transition.Error!.Code);
        Assert.AreEqual(CredentialActionKind.DeleteRefreshToken, transition.CredentialAction!.Kind);
        Assert.AreNotEqual(CredentialActionKind.StoreRefreshToken, transition.CredentialAction.Kind);
        Assert.AreEqual(AuthSessionState.Unauthenticated, machine.Snapshot.State);
    }

    [TestMethod]
    public void Transient_failures_stop_after_five_retries()
    {
        var machine = ActivatedMachine();
        var request = Heartbeat();
        var now = Now;

        for (var attempt = 1; attempt <= AuthSessionMachine.MaxRetryAttempts; attempt++)
        {
            var key = $"heartbeat-retry-{attempt}";
            _ = machine.BeginHeartbeat(key, request, now);
            var failed = machine.FailHeartbeat(key, new ControlPlaneErrorDto("NETWORK_UNAVAILABLE", "网络不可用", 503), now);
            Assert.AreEqual(AuthTransitionKind.RetryScheduled, failed.Kind);
            now = now.Add(AuthSessionMachine.RetryDelayForAttempt(attempt));
        }

        _ = machine.BeginHeartbeat("heartbeat-retry-exhausted", request, now);
        var exhausted = machine.FailHeartbeat(
            "heartbeat-retry-exhausted",
            new ControlPlaneErrorDto("NETWORK_UNAVAILABLE", "网络不可用", 503),
            now);

        Assert.AreEqual(AuthTransitionKind.Rejected, exhausted.Kind);
        Assert.AreEqual(AuthErrorCodes.RetryExhausted, exhausted.Error!.Code);
        Assert.IsNull(exhausted.Snapshot.NextRetryAt);
    }

    [TestMethod]
    public void Logout_is_local_idempotent_and_reports_unconfirmed_remote_revoke()
    {
        var machine = ActivatedMachine();
        _ = machine.BeginLogout("logout-1", Now);

        var first = machine.CompleteLogout("logout-1", remoteConfirmed: false, now: Now);
        var replay = machine.CompleteLogout("logout-1", remoteConfirmed: false, now: Now.AddSeconds(1));

        Assert.AreEqual(AuthSessionState.Unauthenticated, machine.Snapshot.State);
        Assert.AreEqual(CredentialActionKind.DeleteRefreshToken, first.CredentialAction!.Kind);
        Assert.IsTrue(first.RequiresRemoteLogoutRetry);
        Assert.AreEqual(AuthTransitionKind.Duplicate, replay.Kind);
        Assert.IsTrue(replay.RequiresRemoteLogoutRetry);
    }

    private static AuthSessionMachine LoggedInMachine()
    {
        var machine = new AuthSessionMachine();
        _ = machine.BeginLogin(
            "login-1",
            "device01",
            new DesktopLoginRequestDto("alice", "password-123", ControlPlaneContractValues.Product),
            Now);
        _ = machine.CompleteLogin(
            "login-1",
            new AuthTokenSet("access", "refresh", Now.AddMinutes(15), Now.AddDays(30), ControlPlaneContractValues.DesktopAudience),
            User(),
            Now);
        return machine;
    }

    private static AuthSessionMachine ActivatedMachine()
    {
        var machine = LoggedInMachine();
        _ = machine.BeginActivation("activate-1", Now);
        _ = machine.CompleteActivation("activate-1", ActiveDevice(), Now);
        return machine;
    }

    private static UserSummaryDto User() => new("user-1", "alice", "user", "active", "2026-01-01T00:00:00Z");

    private static DeviceSummaryDto ActiveDevice() => new(
        "device01",
        "user-1",
        ControlPlaneContractValues.Product,
        "Windows desktop",
        "windows",
        "1.0.0",
        "active",
        null,
        null,
        null,
        null,
        "Windows",
        "11",
        "10.0",
        null,
        "stopped",
        true,
        "2026-09-02T08:00:00Z",
        Now.AddDays(30).ToString("O"));

    private static HeartbeatRequestDto Heartbeat() => new(
        ControlPlaneContractValues.Product,
        "device01",
        Now,
        new HeartbeatStatusDto(1024, CurrentMediaName: "sample.mp4", PlaybackState: "idle"));
}
