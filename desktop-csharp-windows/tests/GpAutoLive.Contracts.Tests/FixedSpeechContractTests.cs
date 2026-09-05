using System.Text.Json;
using GpAutoLive.Contracts;

namespace GpAutoLive.Contracts.Tests;

[TestClass]
public sealed class FixedSpeechContractTests
{
    [TestMethod]
    public void Speak_and_cancel_commands_use_the_reference_wire_shape()
    {
        var speak = FixedSpeechCommandDto.Speak("speech-1", "  欢迎来到直播间  ");
        var cancel = FixedSpeechCommandDto.Cancel("speech-1");
        var options = ContractJson.CreateOptions();

        var speakJson = JsonSerializer.Serialize(speak, options);
        var cancelJson = JsonSerializer.Serialize(cancel, options);

        StringAssert.Contains(speakJson, "\"version\":1");
        StringAssert.Contains(speakJson, "\"type\":\"fixed-speech-command\"");
        StringAssert.Contains(speakJson, "\"action\":\"speak\"");
        StringAssert.Contains(speakJson, "\"operation_id\":\"speech-1\"");
        using var speakDocument = JsonDocument.Parse(speakJson);
        Assert.AreEqual("  欢迎来到直播间  ", speakDocument.RootElement.GetProperty("text").GetString());
        StringAssert.Contains(cancelJson, "\"action\":\"cancel\"");
        Assert.IsFalse(cancelJson.Contains("\"text\"", StringComparison.Ordinal));
    }

    [TestMethod]
    public void Strict_json_round_trip_preserves_command_and_status()
    {
        var options = ContractJson.CreateOptions();
        var command = FixedSpeechCommandDto.Speak("speech-2", "你好😀");
        var status = new FixedSpeechStatusDto(
            FixedSpeechContractValues.Version,
            FixedSpeechContractValues.StatusType,
            "speech-2",
            FixedSpeechStatus.Playing,
            null);

        var parsedCommand = JsonSerializer.Deserialize<FixedSpeechCommandDto>(
            JsonSerializer.Serialize(command, options), options);
        var statusJson = JsonSerializer.Serialize(status, options);
        var parsedStatus = JsonSerializer.Deserialize<FixedSpeechStatusDto>(
            statusJson, options);

        Assert.AreEqual(command, parsedCommand);
        Assert.AreEqual(status, parsedStatus);
        StringAssert.Contains(statusJson, "\"status\":\"playing\"");
    }

    [TestMethod]
    public void Command_validation_matches_one_to_five_hundred_unicode_characters()
    {
        Assert.IsTrue(FixedSpeechContractValidation.TryValidateCommand(
            FixedSpeechCommandDto.Speak("speech-3", string.Concat(Enumerable.Repeat("😀", 500))), out _));
        Assert.IsFalse(FixedSpeechContractValidation.TryValidateCommand(
            FixedSpeechCommandDto.Speak("speech-4", string.Concat(Enumerable.Repeat("😀", 501))), out var tooLong));
        Assert.AreEqual(FixedSpeechErrorCodes.TextTooLong, tooLong!.Code);
        Assert.IsFalse(FixedSpeechContractValidation.TryValidateCommand(
            FixedSpeechCommandDto.Speak("speech-5", "   "), out var empty));
        Assert.AreEqual(FixedSpeechErrorCodes.TextRequired, empty!.Code);
    }

    [TestMethod]
    public void Command_and_status_validation_reject_invalid_identity_and_future_shape()
    {
        Assert.IsFalse(FixedSpeechContractValidation.TryValidateCommand(
            FixedSpeechCommandDto.Speak(" ", "有效"), out var emptyId));
        Assert.AreEqual(FixedSpeechErrorCodes.OperationIdInvalid, emptyId!.Code);

        var future = FixedSpeechCommandDto.Speak("speech-6", "有效") with { Version = 2 };
        Assert.IsFalse(FixedSpeechContractValidation.TryValidateCommand(future, out var versionError));
        Assert.AreEqual(FixedSpeechErrorCodes.InvalidCommand, versionError!.Code);

        var invalidStatus = new FixedSpeechStatusDto(
            FixedSpeechContractValues.Version,
            FixedSpeechContractValues.StatusType,
            "speech-6",
            FixedSpeechStatus.Playing,
            string.Concat(Enumerable.Repeat("错", FixedSpeechInputLimits.MaxErrorLength + 1)));
        Assert.IsFalse(FixedSpeechContractValidation.TryValidateStatus(invalidStatus, out var statusError));
        Assert.AreEqual(FixedSpeechErrorCodes.ErrorInvalid, statusError!.Code);
    }
}
