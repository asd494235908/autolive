using System.Text.Json;
using GpAutoLive.Contracts;

namespace GpAutoLive.Contracts.Tests;

[TestClass]
public sealed class InterludeAudioContractTests
{
    [TestMethod]
    public void Defaults_match_reference_low_perception_pool()
    {
        var config = InterludeAudioConfig.Default;

        Assert.AreEqual(20, config.AudioPresetIds.Length);
        Assert.AreEqual("p01", config.AudioFixedPresetId);
        CollectionAssert.AreEqual(
            Enumerable.Range(1, 20).Select(static number => $"p{number:00}").ToArray(),
            config.AudioPresetIds.ToArray());
        Assert.AreEqual(50UL, config.DuckingAttackMs);
        Assert.AreEqual(250UL, config.DuckingReleaseMs);
        Assert.IsTrue(InterludeAudioRules.TryValidate(config, out var error), error?.Message);
    }

    [TestMethod]
    public void Invalid_preset_duplicate_and_order_fail_closed()
    {
        var duplicate = InterludeAudioConfig.Default with { AudioPresetIds = ["p01", "p01"] };
        Assert.IsFalse(InterludeAudioRules.TryValidate(duplicate, out var duplicateError));
        Assert.AreEqual(InterludeAudioConfigFailureCode.AudioPresetDuplicate, duplicateError?.Code);

        var order = InterludeAudioConfig.Default with { AudioMixPickMin = 4, AudioMixPickMax = 1 };
        Assert.IsFalse(InterludeAudioRules.TryValidate(order, out var orderError));
        Assert.AreEqual(InterludeAudioConfigFailureCode.AudioMixPickOrderInvalid, orderError?.Code);

        var attack = InterludeAudioConfig.Default with { DuckingAttackMs = 1_001 };
        Assert.IsFalse(InterludeAudioRules.TryValidate(attack, out var attackError));
        Assert.AreEqual(InterludeAudioConfigFailureCode.DuckingAttackOutOfRange, attackError?.Code);
    }

    [TestMethod]
    public void Config_serializes_as_strict_snake_case_without_sensitive_fields()
    {
        var json = JsonSerializer.Serialize(InterludeAudioConfig.Default, ContractJson.CreateOptions());

        StringAssert.Contains(json, "audio_fixed_preset_id");
        StringAssert.Contains(json, "ducking_release_ms");
        Assert.IsFalse(json.Contains("target_url", StringComparison.OrdinalIgnoreCase));
        Assert.IsFalse(json.Contains("token", StringComparison.OrdinalIgnoreCase));
    }
}
