namespace GpAutoLive.Core.Configuration;

/// <summary>配置输入不符合当前版本契约时使用的稳定错误。</summary>
public sealed class ConfigurationValidationException : Exception
{
    /// <summary>使用稳定配置错误消息创建异常。</summary>
    public ConfigurationValidationException(string message)
        : base(message)
    {
    }

    /// <summary>使用稳定消息和原始异常创建配置错误。</summary>
    public ConfigurationValidationException(string message, Exception innerException)
        : base(message, innerException)
    {
    }
}
