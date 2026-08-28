import { SoundOutlined } from "@ant-design/icons";
import { Alert, Button, Checkbox, Form, Input, Select, Space, Tag } from "antd";
import { FeatureDrawerSection, FeatureDrawerShell } from "./FeatureDrawerShell.jsx";

export function FixedSpeechDrawer({ settings, onChange, onClose, onPreview, onSave, open }) {
  const textLength = settings.text?.trim().length || 0;
  const invalid = textLength < 1 || textLength > 500;
  const pauseInterruption = settings.pauseInterruption !== false;

  return (
    <FeatureDrawerShell
      modifierClass="fixed-speech-drawer"
      title="固定话术"
      description="使用本机系统语音朗读固定文案，朗读期间暂停插话并临时静音原声。"
      width="min(560px, 100vw)"
      open={open}
      onClose={onClose}
      summaryLabel="固定话术状态概览"
      summary={(
        <>
          <Tag color="success">本机系统默认语音</Tag>
          <Tag color={invalid ? "warning" : "blue"}>正文 {textLength} / 500 字</Tag>
          <Tag>{pauseInterruption ? "朗读时暂停随机插话" : "朗读时保留随机插话"}</Tag>
        </>
      )}
      footer={(
        <Button type="primary" disabled={invalid} onClick={onSave}>
          保存本地文案
        </Button>
      )}
    >
      <FeatureDrawerSection title="文案设置" description="选择预制文案并填写便于识别的本地标题。">
        <Form layout="vertical" className="feature-form fixed-speech-form">
          <Form.Item label="预制文案">
            <Select value="welcome" options={[{ value: "welcome", label: "开场欢迎" }, { value: "follow", label: "关注提醒" }]} />
          </Form.Item>
          <Form.Item label="预制标题" style={{ marginBottom: 0 }}>
            <Input value={settings.title} maxLength={80} showCount onChange={(event) => onChange?.("title", event.target.value)} />
          </Form.Item>
        </Form>
      </FeatureDrawerSection>

      <FeatureDrawerSection title="朗读正文" description="正文保存在本地；试听与正式朗读均使用本机系统语音。">
        <Alert type="success" showIcon message="仅调用本机系统语音，不下载模型、不识别人声。" description="朗读时可暂停插话并临时静音原声，完成后自动恢复。" />
        <Form layout="vertical" className="feature-form fixed-speech-form fixed-speech-form--body">
          <Form.Item required label="朗读正文" validateStatus={invalid ? "error" : "success"} help={invalid ? "请输入 1–500 个字符" : "正文长度有效"}>
            <Input.TextArea value={settings.text} maxLength={500} showCount autoSize={{ minRows: 7, maxRows: 12 }} onChange={(event) => onChange?.("text", event.target.value)} />
          </Form.Item>
          <Space direction="vertical" size={10} style={{ width: "100%" }}>
            <Checkbox checked={pauseInterruption} onChange={(event) => onChange?.("pauseInterruption", event.target.checked)}>朗读时暂停随机插话</Checkbox>
            <Button icon={<SoundOutlined />} disabled={invalid} onClick={onPreview}>播放当前文案</Button>
          </Space>
        </Form>
      </FeatureDrawerSection>
    </FeatureDrawerShell>
  );
}

export default FixedSpeechDrawer;
