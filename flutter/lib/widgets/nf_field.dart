// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';

import '../theme.dart';

class NfField extends StatelessWidget {
  const NfField({
    super.key,
    required this.controller,
    this.label,
    this.hint,
    this.obscure = false,
    this.keyboardType,
    this.textInputAction,
    this.onSubmitted,
    this.onChanged,
    this.autofillHints,
    this.autofocus = false,
    this.prefixIcon,
    this.suffix,
    this.autocorrect = true,
    this.maxLines = 1,
  });

  final TextEditingController controller;
  final String? label;
  final String? hint;
  final bool obscure;
  final TextInputType? keyboardType;
  final TextInputAction? textInputAction;
  final ValueChanged<String>? onSubmitted;
  final ValueChanged<String>? onChanged;
  final Iterable<String>? autofillHints;
  final bool autofocus;
  final Widget? prefixIcon;
  final Widget? suffix;
  final bool autocorrect;
  final int maxLines;

  @override
  Widget build(BuildContext context) {
    OutlineInputBorder border(Color color) => OutlineInputBorder(
      borderRadius: BorderRadius.circular(NfRadii.m),
      borderSide: BorderSide(color: color),
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        if (label != null) ...[
          Text(label!, style: nfText(13, weight: 600, color: NfColors.text2)),
          const SizedBox(height: 6),
        ],
        TextField(
          controller: controller,
          obscureText: obscure,
          keyboardType: keyboardType,
          textInputAction: textInputAction,
          onSubmitted: onSubmitted,
          onChanged: onChanged,
          autofillHints: autofillHints,
          autofocus: autofocus,
          autocorrect: autocorrect,
          enableSuggestions: autocorrect,
          maxLines: maxLines,
          style: NfType.body,
          decoration: InputDecoration(
            hintText: hint,
            hintStyle: nfText(15, color: NfColors.text3),
            isDense: true,
            filled: true,
            fillColor: NfColors.surface,
            prefixIcon: prefixIcon,
            suffixIcon: suffix,
            contentPadding: const EdgeInsets.symmetric(
              horizontal: 14,
              vertical: 13,
            ),
            enabledBorder: border(NfColors.lineStrong),
            focusedBorder: border(NfColors.focus),
          ),
        ),
      ],
    );
  }
}
