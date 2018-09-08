'use strict'

const eventEmitter = require('events')
const path = require('path')

const _ = require('lodash')
const walk = require('fs-walk')
const {Int64BE} = require('int64-buffer')

const javaProcess = require('./java_process')
const javaSyntax = require('../../languages/java/syntax')
const javaJNI = require('../../languages/java/jni')

class JavaDebugger extends eventEmitter {
  constructor (progName, args, options) {
    super()

    this.javaProcess = new javaProcess.JavaProcess(progName, args)

    this._pendingBreakpointMethodForClasses = {}

    this._currentThreadID = Buffer.from([
      0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01
    ])

    this._handleJavaEventCommand = this._handleJavaEventCommand.bind(this)
    this._getClassesWithGeneric = this._getClassesWithGeneric.bind(this)
    this._getMethodsWithGeneric = this._getMethodsWithGeneric.bind(this)
    this._setBreakpoint = this._setBreakpoint.bind(this)
    this._breakOnClassPrepare = this._breakOnClassPrepare.bind(this)
    this._handleClassPrepareEvent = this._handleClassPrepareEvent.bind(this)
    this._handleLocationEvent = this._handleLocationEvent.bind(this)
    this._getMethodLineNumbers = this._getMethodLineNumbers.bind(this)

    this.allJavaFiles = new Set()
  }

  setup () {
    this.javaProcess.on('padre_log', (level, str) => {
      this.emit('padre_log', level, str)
    })

    for (let dir of ['./', '/Users/stevent/code/third_party/java', '/Users/stevent/code/third_party/apache-maven-3.5.4']) {
      walk.filesSync(dir, (basedir, filename) => {
        this.allJavaFiles.add(path.normalize(`${basedir}/${filename}`))
      })
    }

    this.emit('started')
  }

  async run () {
    console.log('Java: Run')

    this.javaProcess.on('request', async (commandSet, command, data) => {
      if (commandSet === 64 && command === 100) {
        return this._handleJavaEventCommand(data)
      }
      console.log('REQUEST TODO -')
      console.log({
        'commandSet': commandSet,
        'command': command,
        'data': data,
      })
    })

    this.javaProcess.run()

    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(new Error('Timeout starting node process'))
      }, 5000)

      this.on('jvmstarted', async () => {
        clearTimeout(timeout)

        // await this.javaProcess.request(15, 1, Buffer.concat([
        //   Buffer.from([0x08, 0x02]), // Suspend all on CLASS_PREPARE
        //   Buffer.from([0x00, 0x00, 0x00, 0x00]), // 0 modifiers
        // ]))

        resolve({
          'pid': 0
        })
      })
    })
  }

  async breakpointFileAndLine (file, line) {
    console.log(`Java: Breakpoint at ${file}:${line}`)

    if (!file.endsWith('.java')) {
      this.emit('padre_log', 2, `Bad Filename: ${file}`)
      return
    }

    return new Promise(async (resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(new Error('Timeout setting breakpoint'))
      }, 5000)

      const [classes, positionData] = await Promise.all([
        this._getClassesWithGeneric(),
        javaSyntax.getPositionDataAtLine(file, line)
      ])

      const className = positionData[0]
      const methodName = positionData[1]
      const classSignature = javaJNI.convertClassToJNISignature(className)

      const classFound = _.get(classes.filter(x => x.signature === classSignature), '[0]')

      let status = 'OK'

      if (classFound) {
        await this._setBreakpoint(classFound.refTypeID, methodName)
      } else {
        await this._breakOnClassPrepare(className)
        this._pendingBreakpointMethodForClasses[classSignature] = methodName
        status = 'PENDING'
      }

      clearTimeout(timeout)
      resolve({
        'status': status
      })
    })
  }

  async stepIn () {
    console.log('Java: StepIn')

    return new Promise(async (resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(new Error('Timeout stepping in'))
      }, 5000)

      // TODO: Error handle
      await this.javaProcess.request(15, 1, Buffer.concat([
        Buffer.from([0x01]), // SINGLE_STEP EventKind
        Buffer.from([0x02]), // Suspend All
        Buffer.from([0x00, 0x00, 0x00, 0x06]), // 6 Modifiers
        Buffer.from([0x0a]), // Step modKind
        this._currentThreadID,
        Buffer.from([0x00, 0x00, 0x00, 0x01]), // Size LINE
        Buffer.from([0x00, 0x00, 0x00, 0x00]), // Into
        Buffer.from([0x06]), // Class Exclude (java.*)
        Buffer.from([0x00, 0x00, 0x00, 0x06, 0x6a, 0x61, 0x76, 0x61, 0x2e, 0x2a]),
        Buffer.from([0x06]), // Class Exclude (javax.*)
        Buffer.from([0x00, 0x00, 0x00, 0x07, 0x6a, 0x61, 0x76, 0x61, 0x78, 0x2e, 0x2a]),
        Buffer.from([0x06]), // Class Exclude (sun.*)
        Buffer.from([0x00, 0x00, 0x00, 0x05, 0x73, 0x75, 0x6e, 0x2e, 0x2a]),
        Buffer.from([0x06]), // Class Exclude (com.sun.*)
        Buffer.from([0x00, 0x00, 0x00, 0x09, 0x63, 0x6f, 0x6d, 0x2e, 0x73, 0x75, 0x6e, 0x2e, 0x2a]),
        Buffer.from([0x01]), // Count: Do it once only
        Buffer.from([0x00, 0x00, 0x00, 0x01]),
      ]))

      await this.javaProcess.request(1, 9)

      clearTimeout(timeout)
      resolve({})
    })
  }

  async stepOver () {
    console.log('Java: StepOver')

    return new Promise(async (resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(new Error('Timeout stepping over'))
      }, 5000)

      // TODO: Error handle
      await this.javaProcess.request(15, 1, Buffer.concat([
        Buffer.from([0x01]), // SINGLE_STEP EventKind
        Buffer.from([0x02]), // Suspend All
        Buffer.from([0x00, 0x00, 0x00, 0x06]), // 6 Modifiers
        Buffer.from([0x0a]), // Step modKind
        this._currentThreadID,
        Buffer.from([0x00, 0x00, 0x00, 0x01]), // Size LINE
        Buffer.from([0x00, 0x00, 0x00, 0x01]), // Over
        Buffer.from([0x06]), // Class Exclude (java.*)
        Buffer.from([0x00, 0x00, 0x00, 0x06, 0x6a, 0x61, 0x76, 0x61, 0x2e, 0x2a]),
        Buffer.from([0x06]), // Class Exclude (javax.*)
        Buffer.from([0x00, 0x00, 0x00, 0x07, 0x6a, 0x61, 0x76, 0x61, 0x78, 0x2e, 0x2a]),
        Buffer.from([0x06]), // Class Exclude (sun.*)
        Buffer.from([0x00, 0x00, 0x00, 0x05, 0x73, 0x75, 0x6e, 0x2e, 0x2a]),
        Buffer.from([0x06]), // Class Exclude (com.sun.*)
        Buffer.from([0x00, 0x00, 0x00, 0x09, 0x63, 0x6f, 0x6d, 0x2e, 0x73, 0x75, 0x6e, 0x2e, 0x2a]),
        Buffer.from([0x01]), // Count 1
        Buffer.from([0x00, 0x00, 0x00, 0x01]),
      ]))

      await this.javaProcess.request(1, 9)

      clearTimeout(timeout)
      resolve({})
    })
  }

  async continue () {
    console.log('Java: Continue')

    const ret = this.javaProcess.request(1, 9)

    return new Promise(async (resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(new Error('Timeout continuing'))
      }, 5000)

      await ret
      clearTimeout(timeout)
      resolve({})
    })
  }

  async printVariable (variableName, file, line) {
    console.log('Java: Print Variable')

    return new Promise(async (resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(new Error('Timeout printing a variable'))
      }, 5000)

      console.log(file)

      const [classes, positionData] = await Promise.all([
        this._getClassesWithGeneric(),
        javaSyntax.getPositionDataAtLine(file, line)
      ])

      const className = positionData[0]
      const classSignature = javaJNI.convertClassToJNISignature(className)
      const classFound = _.get(classes.filter(x => x.signature === classSignature), '[0]')
      const classRefTypeID = classFound.refTypeID

      const methodName = positionData[1]
      const methods = await this._getMethodsWithGeneric(classRefTypeID)
      const methodFound = _.get(methods.filter(x => x.name === methodName), '[0]')
      const methodID = methodFound.methodID

      const variables = await this._getVariablesInMethod(classRefTypeID, methodID)
      const variable = variables.filter(x => x.variableName === variableName)[0]

      let {
        type,
        value,
      } = await this._getValueForVariable(variable.slot, variable.signature)

      switch (type) {
      case 'B':
      case 'C':
      case 'D':
      case 'F':
      case 'I':
      case 'S':
        type = 'number'
        break
      case 'Z':
        type = 'boolean'
        break
      case 'L':
        type = 'string'
        break
      case 's':
        type = 'string'
        value = await this._getStringValue(value)
        break
      }

      clearTimeout(timeout)
      resolve({
        'type': type,
        'value': value,
        'variable': variableName,
      })
    })
  }

  async _handleJavaEventCommand (data) {
    let pos = 5
    for (let i = 0; i < data.readInt32BE(1); i++) {
      const eventKind = data.readInt8(pos)
      pos += 1
      if (eventKind === 0x01 || eventKind === 0x02) {
        pos += await this._handleLocationEvent(data.slice(pos))
      } else if (eventKind === 0x08) {
        pos += await this._handleClassPrepareEvent(data.slice(pos))
      } else if (eventKind === 0x5a) {
        this.emit('jvmstarted')
        pos += 12
      } else if (eventKind === 0x63) {
        this.emit('process_exit', 0, 0) // TODO: Exit Codes
        pos += 4
      } else {
        console.log('TODO: Handle eventKind ' + eventKind)
        console.log(data.slice(pos))
      }
    }
  }

  async _getClassesWithGeneric () {
    const ret = await this.javaProcess.request(1, 20)
    // TODO: Error Handle
    if (ret.errorCode !== 0) {
      console.log('Get all classes errorCode - ' + ret.errorCode)
    }
    const data = ret.data

    let pos = 4
    let classes = []

    for (let i = 0; i < data.readInt32BE(0); i++) {
      const clazz = {}

      clazz.refTypeTag = data.readInt8(pos)
      pos += 1

      clazz.refTypeID = data.slice(pos, pos + this.javaProcess.getReferenceTypeIDSize())
      pos += this.javaProcess.getReferenceTypeIDSize()

      const signatureSize = data.readInt32BE(pos)
      pos += 4
      clazz.signature = data.slice(pos, pos + signatureSize).toString('utf-8')
      pos += signatureSize

      const genericSignatureSize = data.readInt32BE(pos)
      pos += 4
      clazz.genericSignature = data.slice(pos, pos + genericSignatureSize).toString('utf-8')
      pos += genericSignatureSize

      clazz.status = data.readInt32BE(pos)
      pos += 4
      classes.push(clazz)
    }

    return classes
  }

  async _getMethodsWithGeneric (refTypeID) {
    const ret = await this.javaProcess.request(2, 15, refTypeID)
    // TODO: Error Handle
    if (ret.errorCode !== 0) {
      console.log('Get methods errorCode - ' + ret.errorCode)
    }
    const data = ret.data

    let pos = 4
    let methods = []

    for (let i = 0; i < data.readInt32BE(0); i++) {
      const method = {}

      method.methodID = data.slice(pos, pos + 8)
      pos += 8

      const methodNameSize = data.readInt32BE(pos)
      pos += 4
      method.name = data.slice(pos, pos + methodNameSize).toString('utf-8')
      pos += methodNameSize

      const signatureSize = data.readInt32BE(pos)
      pos += 4
      method.signature = data.slice(pos, pos + signatureSize).toString('utf-8')
      pos += signatureSize

      const genericSignatureSize = data.readInt32BE(pos)
      pos += 4
      method.genericSignature = data.slice(pos, pos + genericSignatureSize).toString('utf-8')
      pos += genericSignatureSize

      method.modBits = data.readInt32BE(pos)
      pos += 4
      methods.push(method)
    }

    return methods
  }

  async _setBreakpoint (refTypeID, methodName) {
    const methods = await this._getMethodsWithGeneric(refTypeID)
    const methodFound = _.get(methods.filter(x => x.name === methodName), '[0]')

    // TODO: If not methodFound??

    await this.javaProcess.request(15, 1, Buffer.concat([
      Buffer.from([0x02, 0x02]),
      Buffer.from([0x00, 0x00, 0x00, 0x01]),
      Buffer.from([0x07]),
      Buffer.from([0x01]),
      refTypeID,
      methodFound.methodID,
      Buffer.from([0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
    ]))
  }

  async _breakOnClassPrepare (className) {
    console.log(`Setting break on class prepare for ${className}`)
    let length = Buffer.from([0x00, 0x00, 0x00, 0x00])
    length.writeInt32BE(Buffer.from(className).length) // TODO: Correct length for Unicode?
    await this.javaProcess.request(15, 1, Buffer.concat([
      Buffer.from([0x08, 0x02]), // Suspend all on CLASS_PREPARE
      Buffer.from([0x00, 0x00, 0x00, 0x02]), // 2 modifiers
      Buffer.from([0x05]), // Class pattern to match
      length,
      Buffer.from(className),
      Buffer.from([0x01]), // Count 1
      Buffer.from([0x00, 0x00, 0x00, 0x01])
    ]))
  }

  async _handleClassPrepareEvent (data) {
    let pos = 0

    pos += 4
    pos += this.javaProcess.getObjectIDSize()
    pos += 1

    const refTypeID = data.slice(pos, pos + this.javaProcess.getReferenceTypeIDSize())
    pos += this.javaProcess.getReferenceTypeIDSize()

    //const classPath = await this._getPathForClass(refTypeID)
    //if (!classPath) {
    //  return
    //}

    const signatureSize = data.readInt32BE(pos)
    pos += 4
    const classSignature = data.slice(pos, pos + signatureSize).toString('utf-8')
    pos += signatureSize

    pos += 4

    // const methods = await this._getMethodsWithGeneric(refTypeID)
    //
    // const promises = []
    //
    // for (let method of methods) {
    //   promises.push(this.javaProcess.request(15, 1, Buffer.concat([
    //     Buffer.from([0x02, 0x02]),
    //     Buffer.from([0x00, 0x00, 0x00, 0x01]),
    //     Buffer.from([0x07]),
    //     Buffer.from([0x01]),
    //     refTypeID,
    //     method.methodID,
    //     Buffer.from([0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
    //   ])))
    // }
    //
    // await Promise.all(promises)

    await this._setBreakpoint(refTypeID, this._pendingBreakpointMethodForClasses[classSignature])

    // TODO: Clear Class Prepare Event

    delete this._pendingBreakpointMethodForClasses[classSignature]

    await this.javaProcess.request(1, 9)

    return pos
  }

  async _handleLocationEvent (data) {
    // TODO: Get lengths from correct place

    this._currentThreadID = data.slice(4, 12)
    const classID = data.slice(13, 21)
    const methodID = data.slice(21, 29)
    const location = data.slice(29, 37)

    const [classPath, methodLines] = await Promise.all([
      this._getPathForClass(classID),
      this._getMethodLineNumbers(classID, methodID),
    ])

    if (!classPath) {
      return
    }

    const line = _.get(_.last(methodLines[2].filter(x => {
      // Loop over every index and compare in order, to check whether the current
      // line is less than or equal to the location. e.g.
      // 00 01 02 03 04 05 06 07 < 00 02 03 04 05 06 07 08 and
      // 00 01 02 03 04 05 06 07 == 00 01 02 03 04 05 06 07 and
      for (let i = 0; i < 8; i++) {
        if (location[i] < x.lineCodeIndex[i]) {
          return false
        }
      }
      return true
    })), 'lineNumber')

    this.emit('process_position', classPath, line)
  }

  async _getPathForClass (classID) {
    const [classes, sourceFile] = await Promise.all([
      this._getClassesWithGeneric(),
      this.javaProcess.request(2, 7, classID)
    ])

    if (sourceFile.errorCode !== 0) {
      return null
    }

    const classFileSize = sourceFile.data.readInt32BE()
    const classFile = sourceFile.data.slice(4, 4 + classFileSize).toString('utf-8')
    const classFound = _.get(classes.filter(x => x.refTypeID.equals(classID)), '[0]')
    const fullClassPath = classFound.signature.substr(1, classFound.signature.lastIndexOf('/')) + classFile

    const classPathsFound = [...this.allJavaFiles].filter(x => x.indexOf(fullClassPath) !== -1)
    if (classPathsFound.length === 0) {
      await this.javaProcess.request(1, 9)
      return null
    }

    // TODO: error logging

    if (classPathsFound.length > 1) {
      console.log('TODO: Figure out what to do with:')
      console.log(JSON.stringify(classPathsFound))
    }

    return _.get(classPathsFound, '[0]')
  }

  async _getMethodLineNumbers (classID, methodID) {
    const ret = await this.javaProcess.request(6, 1, Buffer.concat([classID, methodID]))
    // TODO: Error Handle
    if (ret.errorCode !== 0) {
      console.log('Get methods lines errorCode - ' + ret.errorCode)
    }
    const data = ret.data

    const startLine = data.slice(0, 8)
    const endLine = data.slice(8, 16)
    const lineNumbers = []

    let pos = 20
    for (let lineNum = 0; lineNum < data.readInt32BE(16); lineNum++) {
      const lineCodeIndex = data.slice(pos, pos + 8)
      pos += 8
      const lineNumber = data.readInt32BE(pos)
      pos += 4
      lineNumbers.push({
        'lineCodeIndex': lineCodeIndex,
        'lineNumber': lineNumber,
      })
    }

    return [startLine, endLine, lineNumbers]
  }

  async _getVariablesInMethod (classID, methodID) {
    const ret = await this.javaProcess.request(6, 5, Buffer.concat([classID, methodID]))
    // TODO: Error Handle
    if (ret.errorCode !== 0) {
      console.log('Get variables in method errorCode - ' + ret.errorCode)
    }
    const data = ret.data

    let variables = []

    let pos = 8
    for (let i = 0; i < data.readInt32BE(4); i++) {
      const a = {}
      a.codeIndex = data.slice(pos, pos + 8)
      pos += 8

      const variableNameSize = data.readInt32BE(pos)
      pos += 4
      a.variableName = data.slice(pos, pos + variableNameSize).toString('utf-8')
      pos += variableNameSize

      const signatureSize = data.readInt32BE(pos)
      pos += 4
      a.signature = data.slice(pos, pos + signatureSize).toString('utf-8')
      pos += signatureSize

      const genericSignatureSize = data.readInt32BE(pos)
      pos += 4
      a.genericSignature = data.slice(pos, pos + genericSignatureSize).toString('utf-8')
      pos += genericSignatureSize

      a.length = data.readInt32BE(pos)
      pos += 4

      a.slot = data.readInt32BE(pos)
      pos += 4

      variables.push(a)
    }

    return variables
  }

  async _getValueForVariable (slot, signature) {
    let ret = await this.javaProcess.request(11, 6, Buffer.concat([
      this._currentThreadID,
      Buffer.from([0x00, 0x00, 0x00, 0x00]),
      Buffer.from([0x00, 0x00, 0x00, 0x01]),
    ]))
    // TODO: Error Handle
    if (ret.errorCode !== 0) {
      console.log('11, 6 errorCode - ' + ret.errorCode)
    }

    let frameID = ret.data.slice(4, 12)

    let slotBuffer = Buffer.alloc(4)
    slotBuffer.writeInt32BE(slot)

    if (signature === 'Ljava/lang/String;') {
      signature = 's'
    }

    ret = await this.javaProcess.request(16, 1, Buffer.concat([
      this._currentThreadID,
      frameID,
      Buffer.from([0x00, 0x00, 0x00, 0x01]),
      slotBuffer,
      Buffer.from(signature)
    ]))
    // TODO: Error Handle
    if (ret.errorCode !== 0) {
      console.log('16, 1 errorCode - ' + ret.errorCode)
    }

    if (ret.data.readInt32BE() !== 1) {
      console.log('TODO: We have an error')
      console.log(ret)
    }

    const type = ret.data.slice(4, 5).toString('utf-8')
    let value = ret.data.slice(5)

    switch (type) {
    case 'B':
      value = ret.data.readInt8(5)
      break
    case 'C':
    case 'S':
      value = ret.data.readInt16BE(5)
      break
    case 'D':
      value = ret.data.readDoubleBE(5)
      break
    case 'F':
      value = ret.data.readFloatBE(5)
      break
    case 'I':
      value = ret.data.readInt32BE(5)
      break
    case 'L':
      const longValue = new Int64BE(ret.data.slice(5))
      value = longValue.toString(10)
      break
    case 'Z':
      value = ret.data.readInt8(5) !== 0
      break
    }

    return {
      'type': type,
      'value': value,
    }
  }

  async _getStringValue (objectID) {
    console.log(objectID)
    const ret = await this.javaProcess.request(10, 1, Buffer.concat([
      objectID,
    ]))
    // TODO: Error Handle
    if (ret.errorCode !== 0) {
      console.log('10, 1 errorCode - ' + ret.errorCode)
    }

    return ret.data.slice(4).toString('utf-8')
  }

  // async _getClassPaths () {
  //   let ret = await this.javaProcess.request(1, 13)
  //   // TODO: Error Handle
  //   if (ret.errorCode !== 0) {
  //     console.log('Get class paths errorCode - ' + ret.errorCode)
  //   }
  //   const data = ret.data

  //   const baseClassPathSize = data.readInt32BE()
  //   // const baseClassPath = data.slice(4, 4 + baseClassPathSize).toString('utf-8')

  //   let pos = 4 + baseClassPathSize

  //   ret = this._getListFromData(data.slice(pos))
  //   pos += ret[0]
  //   const classPaths = ret[1]

  //   ret = this._getListFromData(data.slice(pos))
  //   pos += ret[0]
  //   const bootClassPaths = ret[1]

  //   return [classPaths, bootClassPaths]
  // }

  // // Takes a Buffer of that contains the following and returns a list of the data
  // //   4 byte integer of the total number of total items
  // //   Repeated data consisting of:
  // //     String length,
  // //     String data
  // _getListFromData (data) {
  //   const numElements = data.readInt32BE()

  //   let ret = []
  //   let pos = 4

  //   for (let i = 0; i < numElements; i++) {
  //     const elementLength = data.readInt32BE(pos)
  //     pos += 4
  //     ret.push(data.slice(pos, pos + elementLength).toString('utf-8'))
  //     pos += elementLength
  //   }

  //   return [pos, ret]
  // }
}

module.exports = {
  JavaDebugger
}
