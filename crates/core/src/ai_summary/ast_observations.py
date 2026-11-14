"""将字面量 AST 的块内文本表投影为可查询记录，不解释宏或条件执行语义。

Skill 的顶层 text 表导出与一些 AST 的块内 text/select 表是两种结构。
保留原树和原始命令，投影只解决数据入口，不把字符串引用当作执行证明。
"""


def numeric(key):
    return type(key) is int or isinstance(key, str) and key.isdigit()


def project(tree, source):
    blocks, dialogue, choices = [], [], []
    for key, block in tree.items():
        if not isinstance(block, dict) or not (numeric(key) or str(key).startswith('block_')):
            continue
        location = dict(file=source, block=str(key), source_line=block.get('line'),
                        pointer='/ast_tree/'+str(key), runtime_verified=False)
        commands = [dict(index=str(i), arguments=command) for i, command in block.items()
                    if numeric(i) and isinstance(command, dict)]
        blocks.append(dict(location, commands=commands, linknext=block.get('linknext'),
                           linkback=block.get('linkback'), text_tables=block.get('text'),
                           choice_tables=block.get('select')))
        table = block.get('select')
        if isinstance(table, dict):
            for language, options in table.items():
                if isinstance(options, dict):
                    for index, text in options.items():
                        if numeric(index) and isinstance(text, str):
                            choices.append(dict(location, language_key=language,
                                option_index=int(index), text=text,
                                kind='literal-block-choice', command_mapping='unverified'))
        table = block.get('text')
        if not isinstance(table, dict):
            continue
        for language, messages in table.items():
            if language in ('vo', 'name') or not isinstance(messages, dict):
                continue
            for index, message in messages.items():
                if not numeric(index) or not isinstance(message, (str, dict)):
                    continue
                # 只合并消息记录的直接字面量片段；嵌套格式指令保留在原表中。
                fragments = ([message] if isinstance(message, str) else
                             [v for k, v in message.items() if numeric(k) and isinstance(v, str)])
                if fragments:
                    dialogue.append(dict(location, language_key=language, message_index=int(index),
                        text=''.join(fragments), speaker=None,
                        speaker_fields=message.get('name') if isinstance(message, dict) else None,
                        kind='literal-block-message', consumer_order='unverified'))
    return dict(block_observations=blocks, choice_texts=choices, dialogue=dialogue)
