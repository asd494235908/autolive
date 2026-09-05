using System.Text.Json;
using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>跨进程 JSON 的唯一基础选项；调用方按消息边界创建实例。</summary>
public static class ContractJson
{
    /// <summary>创建严格的 snake_case JSON 选项，并将枚举写成字符串。</summary>
    public static JsonSerializerOptions CreateOptions()
    {
        var options = new JsonSerializerOptions(JsonSerializerDefaults.Web)
        {
            PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
            DictionaryKeyPolicy = JsonNamingPolicy.SnakeCaseLower,
            UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow
        };

        options.Converters.Add(new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower));
        return options;
    }
}
