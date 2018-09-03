'use strict'

const primitiveTypesMap = {
  'bool': 'Z',
  'byte': 'B',
  'char': 'C',
  'short': 'S',
  'int': 'I',
  'long': 'J',
  'float': 'F',
  'double': 'D',
  'void': 'V'
}

const convertClassToJNISignature = (cls) => {
  return 'L' + cls.replace(/\./g, '/') + ';'
}

const convertMethodToJNISignature = (returnType, args) => {
  let ret = '('
  for (let arg of args) {
    if (arg.endsWith('[]')) {
      ret += '['
      arg = arg.slice(0, arg.indexOf('['))
    }
    if (arg in primitiveTypesMap) {
      ret += primitiveTypesMap[arg]
    } else {
      ret += convertClassToJNISignature(arg)
    }
  }
  ret += ')' + primitiveTypesMap[returnType]
  return ret
}

const convertJNISignatureToClass = (sig) => {
  if (sig[0] !== 'L') {
    throw new Error(`Can't convert '${sig}' to a class`)
  }

  return sig.slice(1).replace(/\//g, '.').replace(/;$/, '')
}

module.exports = {
  convertClassToJNISignature,
  convertMethodToJNISignature,
  convertJNISignatureToClass,
}
